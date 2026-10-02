// SPDX-License-Identifier: GPL-3.0-or-later
//
// Every endpoint the official client declares has a row in the endpoint
// table.
//
// `parity.txt` lists them, one method and path a line with `{}` for each
// parameter — 286, the distinct requests of the official app's interface, an
// interface fact transcribed and nothing more. This test fails if one of them
// stops having a row in `endpoints.rs`: a request that is in the service but
// not checked here is one nobody would notice breaking.

use std::collections::BTreeSet;

fn normalised(route: &str) -> String {
    route
        .trim_start_matches('/')
        .split('/')
        .map(|segment| {
            if segment.chars().all(|c| c.is_ascii_digit()) {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn every_endpoint_of_the_official_client_has_a_row() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let wanted: BTreeSet<String> = std::fs::read_to_string(format!("{dir}/tests/parity.txt"))
        .expect("the list of endpoints")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let table = std::fs::read_to_string(format!("{dir}/tests/endpoints.rs")).expect("the table");

    // Routes appear as `"GET" "/path"` in a row, `method("GET")).and(path("/path"))`
    // in a hand-written test, and as the routes of a loop over several.
    let mut tested = BTreeSet::new();
    let mut method = None::<&str>;
    for token in table.split('"') {
        match token {
            "GET" | "POST" => method = Some(token),
            route if route.starts_with('/') => {
                // A verb belongs to the one route after it. Routes listed in a
                // loop come before their `method("POST")`, so they have none
                // of their own; every such loop in the table is over POSTs.
                let verb = method.take().unwrap_or("POST");
                tested.insert(format!("{verb} {}", normalised(route)));
            }
            _ => {}
        }
    }

    let missing: Vec<&String> = wanted.iter().filter(|w| !tested.contains(*w)).collect();
    assert!(missing.is_empty(), "endpoints without a row: {missing:#?}");
    assert_eq!(wanted.len(), 286, "the official client's count");
}
