#[path = "../support/archive.rs"]
mod archive;

#[test]
fn every_authenticated_scenario_has_the_expected_result() {
    for scenario in 0..archive::SCENARIOS {
        for count in 0..4 {
            archive::run(&[scenario, count, 0]);
            archive::run(&[scenario, count, 0, 1, 2, 3, 4]);
        }
    }
}

#[test]
fn authenticated_chunk_boundaries_and_malformed_trailers() {
    for scenario in [0, 1, 4, 6, 7, 11] {
        for size in [0x80, 0x81, 0x82] {
            archive::run(&[scenario, 1, size, 42]);
        }
    }
}
