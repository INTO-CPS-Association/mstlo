use mstlo::core::Variables;
use mstlo::monitor::{Algorithm, StlMonitor, semantic_markers};
use mstlo::stl;
use mstlo::step;
use mstlo::synchronizer::SynchronizationStrategy;
use std::time::Duration;

fn main() {
    let phi1 = stl!(G[0, 5] temp < $MAX_TEMP);
    let phi2 = stl!(pressure > 10.0 -> F[0, 2] valve_open == 1.0);
    let phi = stl!(phi1 and phi2);

    println!("Formula Structure:\n{}", phi.to_tree_string(2));

    let vars = Variables::new();
    vars.set("MAX_TEMP", 120.0);

    let mut monitor = StlMonitor::builder()
        .formula(phi)
        .semantics(semantic_markers::Rosi)
        .algorithm(Algorithm::Incremental)
        .synchronization_strategy(SynchronizationStrategy::ZeroOrderHold)
        .variables(vars.clone())
        .build()
        .expect("Failed to build STL monitor");

    println!("{}", monitor);

    monitor.update(&step!("temp", 125.5, Duration::from_secs(0)));
    monitor.update(&step!("pressure", 15.0, Duration::from_secs(0)));
    let res = monitor.update(&step!("valve_open", 1.0, Duration::from_secs(0)));
    println!("Verdicts after updates: {}", res);
}
