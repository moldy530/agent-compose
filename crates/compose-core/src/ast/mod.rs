//! The abstract syntax tree of one `agent-compose` file.
//!
//! The AST is a typed, fully spanned rendering of `docs/grammar.md`: one Rust
//! shape per construct the grammar defines, and a [`Span`](crate::diag::Span)
//! on everything a later pass could point a diagnostic at — every definition,
//! every node, every schema field, every mapping key.
//!
//! Two properties are deliberate:
//!
//! * **Partial trees are normal.** Required keys are `Option`, and the forms
//!   that select on a discriminating key carry an `Invalid` variant. A file
//!   with an error still yields a tree, so one bad node does not blind the
//!   compiler to the rest of the file.
//! * **Expressions stay raw.** CEL is kept as written ([`Cel`]); durations,
//!   path expressions, and environment references have lexical grammars the
//!   parser decides, so those carry both the raw text and its decomposition.
//!   Nothing here has been resolved against another file.

pub mod binding;
pub mod common;
pub mod definition;
pub mod deploy;
pub mod document;
pub mod flow;
pub mod policy;
pub mod schema;
pub mod server_tools;
pub mod trigger;

pub use binding::{
    Binding, Bindings, DependencyEntry, ExecBlock, FunctionBinding, HttpBlock, HttpMethod,
    InterpolatedEntry, ModuleBlock, NodeInput, WriteEntry, Writes,
};
pub use common::{
    Address, Cel, ControlTarget, Duration, DurationUnit, EdgeSource, EdgeTarget, EnvRef, Ident,
    Interpolated, Literal, LiteralEntry, Namespace, PSEUDO_NODES, PathExpr, PathStep,
    RESERVED_ROOT_NAMES,
};
pub use definition::{
    AgentAccess, AgentDef, Definition, DefinitionBody, DirectModel, EmbedBlock, ModelDef,
    ProviderDef, ProviderKind, RouteCondition, RouteModel, ServerToolDef, Settings, StoreDef,
    StoreKind, StoreScope, ToolDef, ToolImplementation,
};
pub use deploy::{
    BackendAlias, BackendConfig, BackendDefault, BackendProvider, ConnectionField, EventSource,
    EventSourceKind, EventSourcesSection, HubSection, Placement, PlacementsSection, SECRET_FIELDS,
    StorageBackendsSection, TraceSinkFormat, TraceSinkSection,
};
pub use document::{
    Channel, DEPLOY_SECTIONS, DeployFile, Document, DocumentKind, ImportPath, ImportsSection,
    Reduce, SPEC_SECTIONS, SpecFile, StateSection,
};
pub use flow::{
    Edge, FlowContext, FlowDef, FlowNode, HumanBlock, ItemError, MapBlock, MapDispatch, MapRoute,
    NODE_KIND_KEYS, Node, NodeKind, StoreNode, StoreOp, StoreOpParams, StoreValue,
};
pub use policy::{OnError, PolicyBlock, Retry};
pub use schema::{
    ArrayType, EnumType, Field, FieldMap, Number, ObjectType, ScalarKind, ScalarType, StringFormat,
    Surface, TypeForm, TypeNode, UnionType, UnionVariant,
};
pub use trigger::{
    AuthScheme, BearerAuth, CallbackAllow, CallbackAuth, CallbackHmac, EventTrigger, HmacAlgorithm,
    HmacAuth, HttpTrigger, Respond, ScheduleTrigger, SignatureEncoding, TRIGGER_TYPES, Trigger,
    TriggerKind, TriggerMethod, TriggersSection,
};

#[cfg(test)]
mod keyword_tests {
    use super::*;

    /// Every closed vocabulary spells its variants the way the grammar does.
    ///
    /// The spelling is the whole of what these types carry: the parser reads a
    /// keyword into one and the IR writes one back out, so a variant renamed on
    /// one side and not the other would change the language quietly. Pinning
    /// them here keeps the two directions on one list.
    #[test]
    fn closed_vocabularies_spell_their_variants_as_the_grammar_does() {
        assert_eq!(StoreScope::Execution.as_str(), "execution");
        assert_eq!(StoreScope::Session.as_str(), "session");
        assert_eq!(StoreScope::Global.as_str(), "global");

        assert_eq!(AgentAccess::Read.as_str(), "read");
        assert_eq!(AgentAccess::ReadWrite.as_str(), "read_write");

        assert_eq!(FlowContext::Isolated.as_str(), "isolated");
        assert_eq!(FlowContext::Inherit.as_str(), "inherit");

        assert_eq!(Respond::Sync.as_str(), "sync");
        assert_eq!(Respond::Async.as_str(), "async");

        assert_eq!(HmacAlgorithm::Sha1.as_str(), "sha1");
        assert_eq!(HmacAlgorithm::Sha256.as_str(), "sha256");
        assert_eq!(HmacAlgorithm::Sha512.as_str(), "sha512");
        assert_eq!(HmacAlgorithm::DEFAULT, HmacAlgorithm::Sha256);

        assert_eq!(SignatureEncoding::Hex.as_str(), "hex");
        assert_eq!(SignatureEncoding::Base64.as_str(), "base64");
        assert_eq!(SignatureEncoding::DEFAULT, SignatureEncoding::Hex);

        assert_eq!(schema::Surface::Input.as_str(), "input");
        assert_eq!(schema::Surface::Result.as_str(), "result");
        assert_eq!(schema::Surface::Channel.as_str(), "channel");
    }
}
