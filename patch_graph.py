import re

with open("crates/argos-osint-core/src/recon/graph.rs", "r") as f:
    content = f.read()

# I will use python to carefully edit the rust file.
