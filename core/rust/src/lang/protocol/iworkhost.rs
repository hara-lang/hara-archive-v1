use super::IComponent;
use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.iworkhost",
    name = "IWorkHost",
    parents = ["IComponent"],
    availability = "capability-gated",
    capability = "native-runtime-protocols"
)]
pub trait IWorkHost: IComponent {
    type Work;
    type Input;
    type Options;
    type Reference;
    type Run;

    #[hara_method(value = "work-submit", arity = 4)]
    fn work_submit(
        &self,
        work: Self::Work,
        input: Self::Input,
        options: Self::Options,
    ) -> Self::Run;

    #[hara_method(value = "work-resolve", arity = 2)]
    fn work_resolve(&self, reference: Self::Reference) -> Self::Run;
}
