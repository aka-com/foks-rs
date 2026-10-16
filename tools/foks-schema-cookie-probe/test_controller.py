import importlib.util
import io
import json
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("cookie_controller", pathlib.Path(__file__).with_name("run-local.py"))
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


class ControllerTests(unittest.TestCase):
    def test_failure_never_promoted_by_pragma_diagnostic(self):
        self.assertFalse(controller.result_ok(500, {"passed": False, "pragma_diagnostic": {"allowed": True}}))
        self.assertFalse(controller.result_ok(200, {"passed": True}))
        self.assertFalse(controller.result_ok(500, {"passed": True, "final_table_absent": True}))
        self.assertTrue(controller.result_ok(200, {"passed": True, "final_table_absent": True}))

    def test_response_bound_and_json(self):
        self.assertEqual(controller.bounded_json(io.BytesIO(b'{"passed":false}')), {"passed": False})
        with self.assertRaises(RuntimeError):
            controller.bounded_json(io.BytesIO(b" " * 8193))
        with self.assertRaises(json.JSONDecodeError):
            controller.bounded_json(io.BytesIO(b"not JSON"))


if __name__ == "__main__":
    unittest.main()
