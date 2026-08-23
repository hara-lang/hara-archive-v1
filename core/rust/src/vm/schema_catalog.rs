//! Admission of exact std.typed catalogs before HBC1 programs become usable.
//!
//! HBC1 carries exact schema coordinates but no mutable lookup policy. This
//! module validates a complete catalog manifest, including dependency closure
//! and #901 strongly connected component evidence, before returning a linked
//! program to an embedding caller.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::hbc_schema_links::{decode_linked_program, LinkedProgram, SchemaCoordinate};

const COMPONENT_EPOCH: &str = ":std.typed.catalog/component-v1";
const HASH_PREFIX: &str = "sha256:";
const DIGEST_HEX_LENGTH: usize = 64;

/// One exact admitted catalog entry and its exact direct dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub coordinate: SchemaCoordinate,
    pub dependencies: Vec<SchemaCoordinate>,
}

impl CatalogEntry {
    pub fn new(
        coordinate: SchemaCoordinate,
        dependencies: Vec<SchemaCoordinate>,
    ) -> Result<Self, String> {
        validate_coordinate(&coordinate)?;
        Ok(Self {
            coordinate,
            dependencies: canonical_coordinates(&dependencies, "catalog entry dependencies")?,
        })
    }
}

/// One deterministic strongly connected component from `std.typed.catalog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogComponent {
    pub id: String,
    pub members: Vec<SchemaCoordinate>,
    pub dependencies: Vec<String>,
}

impl CatalogComponent {
    pub fn new(
        id: impl Into<String>,
        members: Vec<SchemaCoordinate>,
        dependencies: Vec<String>,
    ) -> Result<Self, String> {
        let id = id.into();
        validate_hash(&id, "schema catalog component id")?;
        let members = canonical_coordinates(&members, "schema catalog component members")?;
        if members.is_empty() {
            return Err("schema catalog component requires at least one member".into());
        }
        Ok(Self {
            id,
            members,
            dependencies: canonical_component_dependencies(&dependencies)?,
        })
    }
}

/// A catalog whose exact identities, edges, components, and component order
/// have been validated atomically.
#[derive(Debug, Clone)]
pub struct AdmittedCatalog {
    entries: BTreeMap<SchemaCoordinate, CatalogEntry>,
    components: BTreeMap<String, CatalogComponent>,
    owners: BTreeMap<SchemaCoordinate, String>,
    component_order: Vec<String>,
}

impl AdmittedCatalog {
    pub fn entry(&self, coordinate: &SchemaCoordinate) -> Option<&CatalogEntry> {
        self.entries.get(coordinate)
    }

    pub fn component_for(&self, coordinate: &SchemaCoordinate) -> Option<&CatalogComponent> {
        self.owners
            .get(coordinate)
            .and_then(|id| self.components.get(id))
    }

    pub fn component_order(&self) -> &[String] {
        &self.component_order
    }

    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }
}

/// A linked program released only after every exact link and transitive
/// dependency has been admitted.
#[derive(Debug, Clone)]
pub struct AdmittedLinkedProgram {
    pub linked: LinkedProgram,
    pub resolved_coordinates: Vec<SchemaCoordinate>,
}

/// Reproduces the portable #901 component identity exactly:
/// `sha256(pr-str [:std.typed.catalog/component-v1 members])`.
pub fn component_id(members: &[SchemaCoordinate]) -> Result<String, String> {
    let members = canonical_coordinates(members, "schema catalog component members")?;
    if members.is_empty() {
        return Err("schema catalog component requires at least one member".into());
    }
    let mut input = format!("[{COMPONENT_EPOCH} [");
    for (index, coordinate) in members.iter().enumerate() {
        if index > 0 {
            input.push(' ');
        }
        write!(
            &mut input,
            "[:schema :{} {} \"{}\"]",
            coordinate.id, coordinate.version, coordinate.hash
        )
        .expect("writing to String cannot fail");
    }
    input.push_str("]]");
    let digest = Sha256::digest(input.as_bytes());
    let mut output = String::from(HASH_PREFIX);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}

/// Validates a complete catalog manifest without partially admitting entries.
pub fn admit_catalog(
    entries: &[CatalogEntry],
    components: &[CatalogComponent],
) -> Result<AdmittedCatalog, String> {
    let mut entry_index = BTreeMap::new();
    let mut identities = BTreeMap::<(String, u32), String>::new();
    for raw_entry in entries {
        let entry =
            CatalogEntry::new(raw_entry.coordinate.clone(), raw_entry.dependencies.clone())?;
        let identity = (entry.coordinate.id.clone(), entry.coordinate.version);
        if let Some(existing) = identities.insert(identity, entry.coordinate.hash.clone()) {
            if existing == entry.coordinate.hash {
                return Err("schema catalog contains duplicate exact entry".into());
            }
            return Err("schema catalog contains conflicting immutable identity".into());
        }
        if entry_index
            .insert(entry.coordinate.clone(), entry)
            .is_some()
        {
            return Err("schema catalog contains duplicate exact entry".into());
        }
    }

    for entry in entry_index.values() {
        for dependency in &entry.dependencies {
            if !entry_index.contains_key(dependency) {
                return Err(format!(
                    "schema catalog dependency is not admitted: {}",
                    display_coordinate(dependency)
                ));
            }
        }
    }

    let graph = entry_graph(&entry_index);
    let computed_components = strongly_connected_components(&graph);

    let mut component_index = BTreeMap::new();
    let mut owners = BTreeMap::new();
    let mut declared_components = Vec::new();
    for raw_component in components {
        let component = CatalogComponent::new(
            raw_component.id.clone(),
            raw_component.members.clone(),
            raw_component.dependencies.clone(),
        )?;
        let expected_id = component_id(&component.members)?;
        if component.id != expected_id {
            return Err(format!(
                "schema catalog component id mismatch: expected {expected_id}"
            ));
        }
        if component_index
            .insert(component.id.clone(), component.clone())
            .is_some()
        {
            return Err("schema catalog contains duplicate component id".into());
        }
        for member in &component.members {
            if !entry_index.contains_key(member) {
                return Err(format!(
                    "schema catalog component member is not admitted: {}",
                    display_coordinate(member)
                ));
            }
            if owners
                .insert(member.clone(), component.id.clone())
                .is_some()
            {
                return Err(format!(
                    "schema catalog entry belongs to multiple components: {}",
                    display_coordinate(member)
                ));
            }
        }
        declared_components.push(component.members.clone());
    }

    if owners.len() != entry_index.len() {
        let missing = entry_index
            .keys()
            .find(|coordinate| !owners.contains_key(*coordinate))
            .expect("owner count differs only when one entry is missing");
        return Err(format!(
            "schema catalog entry has no component evidence: {}",
            display_coordinate(missing)
        ));
    }

    declared_components.sort();
    if declared_components != computed_components {
        return Err("schema catalog component evidence does not match dependency graph".into());
    }

    for component in component_index.values() {
        let expected = expected_component_dependencies(component, &entry_index, &owners);
        if component.dependencies != expected {
            return Err(format!(
                "schema catalog component dependencies mismatch for {}",
                component.id
            ));
        }
    }

    let component_graph: BTreeMap<String, BTreeSet<String>> = component_index
        .iter()
        .map(|(id, component)| (id.clone(), component.dependencies.iter().cloned().collect()))
        .collect();
    let component_order = dependency_first_order(component_graph)?;

    Ok(AdmittedCatalog {
        entries: entry_index,
        components: component_index,
        owners,
        component_order,
    })
}

/// Decodes HBC1 and releases it only when every linked coordinate and its
/// dependency closure exists in the admitted catalog.
pub fn admit_linked_program(
    artifact: &[u8],
    catalog: &AdmittedCatalog,
) -> Result<AdmittedLinkedProgram, String> {
    let linked = decode_linked_program(artifact)?;
    let mut reachable = BTreeSet::new();
    let mut pending = linked.schema_links.clone();
    while let Some(coordinate) = pending.pop() {
        let Some(entry) = catalog.entry(&coordinate) else {
            return Err(format!(
                "linked bytecode schema coordinate is not admitted: {}",
                display_coordinate(&coordinate)
            ));
        };
        if reachable.insert(coordinate) {
            pending.extend(entry.dependencies.iter().cloned());
        }
    }

    let mut resolved_coordinates = Vec::new();
    for component_id in &catalog.component_order {
        let component = catalog
            .components
            .get(component_id)
            .expect("admitted component order references an existing component");
        for member in &component.members {
            if reachable.contains(member) {
                resolved_coordinates.push(member.clone());
            }
        }
    }

    Ok(AdmittedLinkedProgram {
        linked,
        resolved_coordinates,
    })
}

fn validate_coordinate(coordinate: &SchemaCoordinate) -> Result<(), String> {
    SchemaCoordinate::new(
        coordinate.id.clone(),
        coordinate.version,
        coordinate.hash.clone(),
    )
    .map(|_| ())
}

fn validate_hash(value: &str, label: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix(HASH_PREFIX) else {
        return Err(format!("{label} must use sha256"));
    };
    if digest.len() == DIGEST_HEX_LENGTH
        && digest
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        Ok(())
    } else {
        Err(format!("{label} must be canonical lowercase hex"))
    }
}

fn canonical_coordinates(
    values: &[SchemaCoordinate],
    label: &str,
) -> Result<Vec<SchemaCoordinate>, String> {
    let mut output = values.to_vec();
    output.sort();
    let mut identities = BTreeMap::<(String, u32), String>::new();
    for coordinate in &output {
        validate_coordinate(coordinate)?;
        let identity = (coordinate.id.clone(), coordinate.version);
        if let Some(existing) = identities.insert(identity, coordinate.hash.clone()) {
            if existing == coordinate.hash {
                return Err(format!("{label} contain a duplicate coordinate"));
            }
            return Err(format!("{label} contain conflicting immutable identities"));
        }
    }
    Ok(output)
}

fn canonical_component_dependencies(values: &[String]) -> Result<Vec<String>, String> {
    let mut output = values.to_vec();
    output.sort();
    for value in &output {
        validate_hash(value, "schema catalog component dependency")?;
    }
    if output.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("schema catalog component dependencies contain a duplicate".into());
    }
    Ok(output)
}

fn entry_graph(
    entries: &BTreeMap<SchemaCoordinate, CatalogEntry>,
) -> BTreeMap<SchemaCoordinate, BTreeSet<SchemaCoordinate>> {
    entries
        .iter()
        .map(|(coordinate, entry)| {
            (
                coordinate.clone(),
                entry.dependencies.iter().cloned().collect(),
            )
        })
        .collect()
}

fn strongly_connected_components(
    graph: &BTreeMap<SchemaCoordinate, BTreeSet<SchemaCoordinate>>,
) -> Vec<Vec<SchemaCoordinate>> {
    fn visit_order(
        node: &SchemaCoordinate,
        graph: &BTreeMap<SchemaCoordinate, BTreeSet<SchemaCoordinate>>,
        seen: &mut BTreeSet<SchemaCoordinate>,
        order: &mut Vec<SchemaCoordinate>,
    ) {
        if !seen.insert(node.clone()) {
            return;
        }
        for dependency in graph.get(node).into_iter().flatten() {
            visit_order(dependency, graph, seen, order);
        }
        order.push(node.clone());
    }

    fn visit_component(
        node: &SchemaCoordinate,
        reverse: &BTreeMap<SchemaCoordinate, BTreeSet<SchemaCoordinate>>,
        seen: &mut BTreeSet<SchemaCoordinate>,
        members: &mut Vec<SchemaCoordinate>,
    ) {
        if !seen.insert(node.clone()) {
            return;
        }
        members.push(node.clone());
        for dependency in reverse.get(node).into_iter().flatten() {
            visit_component(dependency, reverse, seen, members);
        }
    }

    let mut reverse = graph
        .keys()
        .cloned()
        .map(|coordinate| (coordinate, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (coordinate, dependencies) in graph {
        for dependency in dependencies {
            reverse
                .get_mut(dependency)
                .expect("validated dependency exists in graph")
                .insert(coordinate.clone());
        }
    }

    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for coordinate in graph.keys() {
        visit_order(coordinate, graph, &mut seen, &mut order);
    }

    seen.clear();
    let mut output = Vec::new();
    while let Some(coordinate) = order.pop() {
        if seen.contains(&coordinate) {
            continue;
        }
        let mut members = Vec::new();
        visit_component(&coordinate, &reverse, &mut seen, &mut members);
        members.sort();
        output.push(members);
    }
    output.sort();
    output
}

fn expected_component_dependencies(
    component: &CatalogComponent,
    entries: &BTreeMap<SchemaCoordinate, CatalogEntry>,
    owners: &BTreeMap<SchemaCoordinate, String>,
) -> Vec<String> {
    let mut output = BTreeSet::new();
    for member in &component.members {
        for dependency in &entries
            .get(member)
            .expect("component member is an admitted entry")
            .dependencies
        {
            let owner = owners
                .get(dependency)
                .expect("admitted dependency has component evidence");
            if owner != &component.id {
                output.insert(owner.clone());
            }
        }
    }
    output.into_iter().collect()
}

fn dependency_first_order(
    mut graph: BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut output = Vec::new();
    while !graph.is_empty() {
        let ready = graph
            .iter()
            .filter_map(|(component, dependencies)| {
                dependencies.is_empty().then_some(component.clone())
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err("schema catalog component graph contains a cycle".into());
        }
        let ready_set = ready.iter().cloned().collect::<BTreeSet<_>>();
        for component in &ready {
            graph.remove(component);
        }
        for dependencies in graph.values_mut() {
            dependencies.retain(|dependency| !ready_set.contains(dependency));
        }
        output.extend(ready);
    }
    Ok(output)
}

fn display_coordinate(coordinate: &SchemaCoordinate) -> String {
    format!(
        "[:schema :{} {} \"{}\"]",
        coordinate.id, coordinate.version, coordinate.hash
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hbc_schema_links::encode_linked_program;
    use crate::vm::compile_source;

    fn coordinate(id: &str, version: u32, digit: char) -> SchemaCoordinate {
        SchemaCoordinate::new(
            id,
            version,
            format!("sha256:{}", digit.to_string().repeat(64)),
        )
        .unwrap()
    }

    fn component(members: Vec<SchemaCoordinate>, dependencies: Vec<String>) -> CatalogComponent {
        CatalogComponent::new(component_id(&members).unwrap(), members, dependencies).unwrap()
    }

    #[test]
    fn component_identity_matches_the_portable_catalog_epoch() {
        let identifier = coordinate("model/id", 1, '1');
        assert_eq!(
            component_id(&[identifier]).unwrap(),
            "sha256:2adf3278d7b517357e21f1ba2be75d3755851f22eadfe7416e9d1b15af57b941"
        );
    }

    #[test]
    fn linked_program_is_released_with_dependency_first_exact_closure() {
        let identifier = coordinate("model/id", 1, '1');
        let profile = coordinate("model/profile", 2, '2');
        let identifier_component = component(vec![identifier.clone()], vec![]);
        let profile_component =
            component(vec![profile.clone()], vec![identifier_component.id.clone()]);
        let catalog = admit_catalog(
            &[
                CatalogEntry::new(identifier.clone(), vec![]).unwrap(),
                CatalogEntry::new(profile.clone(), vec![identifier.clone()]).unwrap(),
            ],
            &[profile_component, identifier_component],
        )
        .unwrap();

        let program = compile_source("(+ 19 23)").unwrap();
        let artifact = encode_linked_program(&program, &[profile.clone()]).unwrap();
        let admitted = admit_linked_program(&artifact, &catalog).unwrap();
        assert_eq!(admitted.linked.schema_links, vec![profile.clone()]);
        assert_eq!(admitted.resolved_coordinates, vec![identifier, profile]);
    }

    #[test]
    fn stale_or_missing_exact_links_fail_before_program_release() {
        let identifier = coordinate("model/id", 1, '1');
        let catalog = admit_catalog(
            &[CatalogEntry::new(identifier.clone(), vec![]).unwrap()],
            &[component(vec![identifier], vec![])],
        )
        .unwrap();
        let stale = coordinate("model/id", 1, '2');
        let program = compile_source("42").unwrap();
        let artifact = encode_linked_program(&program, &[stale]).unwrap();
        assert!(admit_linked_program(&artifact, &catalog)
            .unwrap_err()
            .contains("is not admitted"));
    }

    #[test]
    fn forged_component_evidence_is_rejected_atomically() {
        let identifier = coordinate("model/id", 1, '1');
        let profile = coordinate("model/profile", 2, '2');
        let forged = component(vec![identifier.clone(), profile.clone()], vec![]);
        assert_eq!(
            admit_catalog(
                &[
                    CatalogEntry::new(identifier.clone(), vec![]).unwrap(),
                    CatalogEntry::new(profile, vec![identifier]).unwrap(),
                ],
                &[forged],
            )
            .unwrap_err(),
            "schema catalog component evidence does not match dependency graph"
        );
    }

    #[test]
    fn valid_self_recursion_remains_one_admitted_component() {
        let node = coordinate("tree/node", 1, '3');
        let catalog = admit_catalog(
            &[CatalogEntry::new(node.clone(), vec![node.clone()]).unwrap()],
            &[component(vec![node.clone()], vec![])],
        )
        .unwrap();
        assert_eq!(catalog.component_order().len(), 1);
        assert_eq!(catalog.entries().count(), 1);
        assert_eq!(catalog.component_for(&node).unwrap().members, vec![node]);
    }
}
