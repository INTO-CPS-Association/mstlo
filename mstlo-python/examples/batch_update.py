import mstlo_python as mstlo

# Create a simple monitor for batch processing
phi_batch = mstlo.parse_formula("x > 10.0")
batch_monitor = mstlo.Monitor(phi_batch, semantics="Rosi")

# Prepare batch data: dict mapping signal names to lists of (value, timestamp) tuples
batch_steps = {
    "x": [
        (5.0, 0.0),  # x=5 at t=0 (robustness: 5-10 = -5)
        (15.0, 1.0),  # x=15 at t=1 (robustness: 15-10 = 5)
        (8.0, 2.0),  # x=8 at t=2 (robustness: 8-10 = -2)
        (12.0, 3.0),  # x=12 at t=3 (robustness: 12-10 = 2)
    ]
}

# Process all steps at once
output = batch_monitor.update_batch(batch_steps)

print("Batch Update Results:")
print(output)
