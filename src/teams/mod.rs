//! Ready-made teams (next-077 spec T) and team discovery (spec D): the template and preset data,
//! the free-name rules, and reading what a project is made of. The atomic create lives in
//! `db::team_templates`, the routes in `server::teams`.

pub mod discover;
pub mod names;
pub mod presets;
pub mod templates;
