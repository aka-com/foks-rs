"""Reject timing qualification when wall and monotonic clocks diverge."""


class ClockGuard:
    def __init__(self, monotonic, wall_ns, tolerance_seconds=1.0):
        self.monotonic_anchor = monotonic
        self.wall_anchor_ns = wall_ns
        self.tolerance_seconds = tolerance_seconds
        self.maximum_drift_seconds = 0.0

    def observe(self, monotonic, wall_ns):
        drift = (wall_ns - self.wall_anchor_ns) / 1_000_000_000 - (
            monotonic - self.monotonic_anchor
        )
        self.maximum_drift_seconds = max(self.maximum_drift_seconds, abs(drift))
        return drift

    @property
    def valid(self):
        return self.maximum_drift_seconds <= self.tolerance_seconds
