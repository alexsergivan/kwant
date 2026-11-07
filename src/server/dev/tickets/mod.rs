//! Work from Jira tickets (0.7.6, jira-tickets spec): pickup, row states and the seam with the
//! brief turn. Ownership is in api.md §1.1: Juno owns `source`, `pickup` and `store`; `hooks` has
//! Juno's signatures and Vega's bodies; Vega adds `brief*` and `tools`. On merge, take the union
//! of these `pub mod` lines.

pub mod hooks;
pub mod pickup;
pub mod routes;
pub mod source;
pub mod store;
pub mod work;
