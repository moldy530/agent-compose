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
pub mod trigger;

pub use binding::{
    Binding, Bindings, ExecBlock, FunctionBinding, HttpBlock, HttpMethod, InterpolatedEntry,
    NodeInput, WriteEntry, Writes,
};
pub use common::{
    Address, Cel, ControlTarget, Duration, DurationUnit, EdgeSource, EdgeTarget, EnvRef, Ident,
    Interpolated, Literal, LiteralEntry, Namespace, PSEUDO_NODES, PathExpr, PathStep,
    RESERVED_CHANNEL_NAMES,
};
pub use definition::{
    AgentAccess, AgentDef, Definition, DefinitionBody, DirectModel, EmbedBlock, ModelDef,
    ProviderDef, ProviderKind, RouteCondition, RouteModel, Settings, StoreDef, StoreKind,
    StoreScope, ToolDef, ToolImplementation,
};
pub use deploy::{
    BackendAlias, BackendConfig, BackendDefault, BackendProvider, ConnectionField, EventSource,
    EventSourceKind, EventSourcesSection, Network, Placement, PlacementsSection, Runtime,
    SECRET_FIELDS, StorageBackendsSection,
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
    EventTrigger, HttpTrigger, Respond, ScheduleTrigger, TRIGGER_TYPES, Trigger, TriggerKind,
    TriggerMethod, TriggersSection,
};
