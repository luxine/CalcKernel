# Historical compiler source snapshots

These checksum-pinned product snapshots let the performance replay rebuild
CalcKernel 0.10.0, 0.11.0, and 0.12.0 from a fresh clone of this repository.
Each archive was created from the source commit recorded in the matching
baseline manifest with `git archive`; it contains only compiler product files
needed to build that compiler. Frozen workload fixtures and reference oracles
remain in this repository. The archives contain no tests, Git history, or
repository metadata.

The replay preparer verifies each archive SHA-256, the source version, and the
case and oracle digests before extraction. It initializes a temporary local Git
repository only so the approved 0.10 adapter patches and before/after source
diff can be checked. The report continues to record the original source commit
from the baseline manifest.

The `.sha256` sidecars use archive basenames so they can be checked from this
directory with `shasum -a 256 --check` or `sha256sum --check`.
