#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAvailability {
    Portable,
    CapabilityGated,
    InventoryOnly,
}

impl ProtocolAvailability {
    pub fn is_guest_visible(self) -> bool {
        !matches!(self, Self::InventoryOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolArity {
    Fixed(usize),
    Variadic {
        minimum: usize,
        maximum: Option<usize>,
    },
}

impl ProtocolArity {
    pub fn guest_arity(self) -> usize {
        match self {
            Self::Fixed(arity) => arity,
            Self::Variadic { .. } => usize::MAX,
        }
    }

    pub fn range(self) -> (usize, Option<usize>) {
        match self {
            Self::Fixed(arity) => (arity, Some(arity)),
            Self::Variadic { minimum, maximum } => (minimum, maximum),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMethodDeclaration {
    pub name: &'static str,
    pub rust_name: &'static str,
    pub arity: ProtocolArity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDeclaration {
    pub namespace: &'static str,
    pub name: &'static str,
    pub parents: &'static [&'static str],
    pub availability: ProtocolAvailability,
    pub capability: Option<&'static str>,
    pub methods: &'static [ProtocolMethodDeclaration],
}

impl ProtocolDeclaration {
    pub fn qualified_name(self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    pub fn runtime_name(self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    pub fn method(self, name: &str) -> Option<ProtocolMethodDeclaration> {
        self.methods
            .iter()
            .copied()
            .find(|method| method.name == name)
    }
}

const PROTOCOL_DECLARATIONS: &[ProtocolDeclaration] = &[
    super::iapplicable::IAPPLICABLE_PROTOCOL_DECLARATION,
    super::iassoc::IASSOC_PROTOCOL_DECLARATION,
    super::icas::ICAS_PROTOCOL_DECLARATION,
    super::iclose::ICLOSE_PROTOCOL_DECLARATION,
    super::icoll::ICOLL_PROTOCOL_DECLARATION,
    super::istream::ISTREAM_PROTOCOL_DECLARATION,
    super::istreamwrite::ISTREAMWRITE_PROTOCOL_DECLARATION,
    super::iabort::IABORT_PROTOCOL_DECLARATION,
    super::istreampoll::ISTREAMPOLL_PROTOCOL_DECLARATION,
    super::istreamoffer::ISTREAMOFFER_PROTOCOL_DECLARATION,
    super::iclosed::ICLOSED_PROTOCOL_DECLARATION,
    super::iflush::IFLUSH_PROTOCOL_DECLARATION,
    super::istreamduplex::ISTREAMDUPLEX_PROTOCOL_DECLARATION,
    super::icomponent::ICOMPONENT_PROTOCOL_DECLARATION,
    super::iwork::IWORK_PROTOCOL_DECLARATION,
    super::iworkexecutor::IWORKEXECUTOR_PROTOCOL_DECLARATION,
    super::iworkstore::IWORKSTORE_PROTOCOL_DECLARATION,
    super::iworkref::IWORKREF_PROTOCOL_DECLARATION,
    super::iworkhost::IWORKHOST_PROTOCOL_DECLARATION,
    super::iworkrun::IWORKRUN_PROTOCOL_DECLARATION,
    super::iconj::ICONJ_PROTOCOL_DECLARATION,
    super::icons::ICONS_PROTOCOL_DECLARATION,
    super::icontext::ICONTEXT_PROTOCOL_DECLARATION,
    super::icoroutine::ICOROUTINE_PROTOCOL_DECLARATION,
    super::icontextlifecycle::ICONTEXTLIFECYCLE_PROTOCOL_DECLARATION,
    super::icount::ICOUNT_PROTOCOL_DECLARATION,
    super::ideps::IDEPS_PROTOCOL_DECLARATION,
    super::ideref::IDEREF_PROTOCOL_DECLARATION,
    super::idereftimeout::IDEREFTIMEOUT_PROTOCOL_DECLARATION,
    super::idisplay::IDISPLAY_PROTOCOL_DECLARATION,
    super::idissoc::IDISSOC_PROTOCOL_DECLARATION,
    super::iempty::IEMPTY_PROTOCOL_DECLARATION,
    super::iencodable::IENCODABLE_PROTOCOL_DECLARATION,
    super::iencode::IENCODE_PROTOCOL_DECLARATION,
    super::iencodevisitor::IENCODEVISITOR_PROTOCOL_DECLARATION,
    super::iequality::IEQUALITY_PROTOCOL_DECLARATION,
    super::iexinfo::IEXINFO_PROTOCOL_DECLARATION,
    super::ifind::IFIND_PROTOCOL_DECLARATION,
    super::ifn::IFN_PROTOCOL_DECLARATION,
    super::ihash::IHASH_PROTOCOL_DECLARATION,
    super::ihashcached::IHASHCACHED_PROTOCOL_DECLARATION,
    super::iindexed::IINDEXED_PROTOCOL_DECLARATION,
    super::iindexedkv::IINDEXEDKV_PROTOCOL_DECLARATION,
    super::iinvokein::IINVOKEIN_PROTOCOL_DECLARATION,
    super::iiter::IITER_PROTOCOL_DECLARATION,
    super::iiterator::IITERATOR_PROTOCOL_DECLARATION,
    super::ilookup::ILOOKUP_PROTOCOL_DECLARATION,
    super::imatch::IMATCH_PROTOCOL_DECLARATION,
    super::imetadata::IMETADATA_PROTOCOL_DECLARATION,
    super::istringlike::ISTRINGLIKE_PROTOCOL_DECLARATION,
    super::imutable::IMUTABLE_PROTOCOL_DECLARATION,
    super::inamespaced::INAMESPACED_PROTOCOL_DECLARATION,
    super::inth::INTH_PROTOCOL_DECLARATION,
    super::iofn::IOFN_PROTOCOL_DECLARATION,
    super::iobjtype::IOBJTYPE_PROTOCOL_DECLARATION,
    super::ipair::IPAIR_PROTOCOL_DECLARATION,
    super::ipeekfirst::IPEEKFIRST_PROTOCOL_DECLARATION,
    super::ipeeklast::IPEEKLAST_PROTOCOL_DECLARATION,
    super::ipersistent::IPERSISTENT_PROTOCOL_DECLARATION,
    super::ipromise::IPROMISE_PROTOCOL_DECLARATION,
    super::ipointer::IPOINTER_PROTOCOL_DECLARATION,
    super::ipopfirst::IPOPFIRST_PROTOCOL_DECLARATION,
    super::ipoplast::IPOPLAST_PROTOCOL_DECLARATION,
    super::ipushfirst::IPUSHFIRST_PROTOCOL_DECLARATION,
    super::ipushlast::IPUSHLAST_PROTOCOL_DECLARATION,
    super::irealize::IREALIZE_PROTOCOL_DECLARATION,
    super::ireduce::IREDUCE_PROTOCOL_DECLARATION,
    super::ireset::IRESET_PROTOCOL_DECLARATION,
    super::ispace::ISPACE_PROTOCOL_DECLARATION,
    super::itomutable::ITOMUTABLE_PROTOCOL_DECLARATION,
    super::itopersistent::ITOPERSISTENT_PROTOCOL_DECLARATION,
    super::iwatch::IWATCH_PROTOCOL_DECLARATION,
];

pub fn protocol_declarations() -> &'static [ProtocolDeclaration] {
    PROTOCOL_DECLARATIONS
}

pub fn find_protocol(name: &str) -> Option<ProtocolDeclaration> {
    PROTOCOL_DECLARATIONS.iter().copied().find(|protocol| {
        protocol.name == name
            || protocol.qualified_name() == name
            || protocol.runtime_name() == name
    })
}
