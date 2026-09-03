#[cfg(test)]
#[path = "method_catalog.rs"]
mod method_catalog;
#[cfg(not(test))]
use crate::method_catalog;

const CANDIDATE_SCHEMA: &str = "code-intel-design-proposal-candidate.v1";
const RESULT_SCHEMA: &str = "code-intel-design-proposal.v1";
const CONTEXT_SCHEMA: &str = "code-intel-design-context.v1";
const CONTEXT_TYPE: &str = "design.context";
const CAPABILITY: &str = "advisory.design-proposal.compat";
const METHODS_ROOT: &str = "orchestration/methods";

#[path = "design_proposal_contract_helpers.rs"]
mod helpers;
#[path = "design_proposal_contract_methods.rs"]
mod methods;
#[path = "design_proposal_contract_payload.rs"]
mod payload;
#[path = "design_proposal_contract_shape.rs"]
mod shape;

pub(crate) use helpers::*;
pub(crate) use methods::*;
pub(crate) use payload::*;
pub(crate) use shape::*;
