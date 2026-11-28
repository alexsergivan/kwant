//! pa12 L3: the pocket toolkit keeps one copy of the draw, the vote counts and the text match.
//! A kit that needs them (Secret Santa, gather, quiz, and later `made`) uses the toolkit's copy
//! through `crate::pocket::toolkit` or the old re-exported paths, so a fix lands everywhere. This
//! reads every file under src/ and finds each definition exactly once, in its home.

use kwnt::pocket::{gather, santa, toolkit};
use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// `(path under src/, line number)` of every line defining `fn <name>(` or `fn <name><`.
fn definitions(name: &str) -> Vec<(String, usize)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rs_files(&root, &mut paths);
    paths.sort();
    let (paren, generic) = (format!("fn {name}("), format!("fn {name}<"));
    let mut found = Vec::new();
    for p in &paths {
        let text = std::fs::read_to_string(p).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if code.contains(&paren) || code.contains(&generic) {
                found.push((p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), i + 1));
            }
        }
    }
    found
}

#[test]
fn each_toolkit_function_is_defined_exactly_once_in_its_home() {
    let homes = [
        ("draw_loop", "pocket/toolkit.rs"),
        ("insert_into_loop", "pocket/toolkit.rs"),
        ("count_ranked", "pocket/toolkit.rs"),
        ("count_one_pick", "pocket/toolkit.rs"),
        ("match_text", "kits/quiz/matching.rs"),
        ("normalise", "kits/quiz/matching.rs"),
    ];
    for (name, home) in homes {
        let found = definitions(name);
        assert_eq!(found.len(), 1, "fn {name} must be defined once under src/, found {found:?}");
        assert_eq!(found[0].0, home, "fn {name} lives in src/{home}, found {found:?}");
    }
}

#[test]
fn the_old_paths_are_the_toolkit_items() {
    // Same function items, not copies: a cast to the same fn pointer type compiles only when the
    // signatures match, and the pointers are equal only when they name one function.
    type Draw = fn(usize, &dyn Fn(usize, usize) -> bool, &mut toolkit::Rng) -> Result<Vec<usize>, toolkit::Block>;
    assert_eq!(santa::draw_loop as Draw as usize, toolkit::draw_loop as Draw as usize);
    type Count = fn(&[Vec<usize>], &[usize]) -> toolkit::Tally;
    assert_eq!(gather::count_ranked as Count as usize, toolkit::count_ranked as Count as usize);
    assert_eq!(gather::count_one_pick as Count as usize, toolkit::count_one_pick as Count as usize);
    assert_eq!(santa::MIN_PEOPLE, toolkit::MIN_PEOPLE);
    // The types are shared too: a toolkit Rng drives santa's draw and gather's Tally is the toolkit's.
    let apart = |_: usize, _: usize| false;
    let order = santa::draw_loop(4, &apart, &mut toolkit::Rng::seeded(7)).unwrap();
    assert_eq!(order, toolkit::draw_loop(4, &apart, &mut santa::Rng::seeded(7)).unwrap(), "same seed, same loop");
    let t: gather::Tally = toolkit::count_one_pick(&[vec![1], vec![1], vec![0]], &[0, 1]);
    assert_eq!(t.outcome, toolkit::Outcome::Winner { option: 1 });
    // And the quiz's text match is reachable from the toolkit.
    assert_eq!(toolkit::normalise("The U.S.A."), "usa");
    assert_eq!(toolkit::match_text("Lionel Messi", "lionel messi", &[]), toolkit::TextMatch::Correct);
}
