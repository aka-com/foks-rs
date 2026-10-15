import unittest
from clock_guard import ClockGuard


class ClockGuardTests(unittest.TestCase):
    def test_equal_elapsed_clocks_remain_valid(self):
        guard = ClockGuard(10.0, 100_000_000_000)
        self.assertEqual(guard.observe(42.0, 132_000_000_000), 0.0)
        self.assertTrue(guard.valid)

    def test_forward_wall_jump_or_excluded_suspend_invalidates(self):
        guard = ClockGuard(10.0, 100_000_000_000)
        self.assertEqual(guard.observe(11.0, 1001_000_000_000), 900.0)
        self.assertFalse(guard.valid)

    def test_backward_wall_jump_invalidates(self):
        guard = ClockGuard(10.0, 100_000_000_000)
        self.assertEqual(guard.observe(12.0, 99_000_000_000), -3.0)
        self.assertFalse(guard.valid)

    def test_later_return_to_anchor_does_not_erase_invalidity(self):
        guard = ClockGuard(10.0, 100_000_000_000)
        guard.observe(11.0, 103_000_000_000)
        guard.observe(12.0, 102_000_000_000)
        self.assertFalse(guard.valid)

    def test_tolerance_boundary_is_inclusive(self):
        guard = ClockGuard(10.0, 100_000_000_000)
        guard.observe(11.0, 102_000_000_000)
        self.assertTrue(guard.valid)
        guard.observe(11.0, 102_000_000_001)
        self.assertFalse(guard.valid)


if __name__ == "__main__":
    unittest.main()
