import mstlo_python as mstlo

vars = mstlo.Variables()
vars.set("threshold", 0.5)
phi = mstlo.parse_formula("G[0,4](x > $threshold) U[0,3] (y < $threshold)")

print(f"Monitoring Formula: {phi}")

# 2. Create the Monitor (using Robustness Semantics)
monitor = mstlo.Monitor(
    phi, semantics="Rosi", synchronization="ZeroOrderHold", variables=vars
)

print(monitor)

# Get signal identifiers used in the formula
signal_ids = monitor.get_signal_identifiers()
print(f"Signal Identifiers in the formula: {signal_ids}")

vals = [
    ("x", 0.0, 0.0),
    ("y", 0.0, 0.0),
    ("x", 1.0, 0.6),
    ("x", 2.0, 0.7),
    ("x", 3.0, 0.4),
    ("x", 4.0, 0.8),
    ("x", 5.0, 0.9),
    ("y", 6.0, -0.6),
    ("y", 7.0, -0.4),
    ("y", 8.0, -0.7),
]

for var, t, val in vals:

    if t == 3.0:
        # Update threshold at runtime
        vars.set("threshold", 0.6)
        print(f"Updated threshold to {vars.get('threshold')} at time {t}")

    # Feed input to the Monitor
    result = monitor.update(var, val, t)

    # Print results
    print(f"At time {t}, after feeding {var}={val}:")
    print(result)
    print("-" * 40)
