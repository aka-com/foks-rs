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

    def test_configuration_requires_complete_selected_gate(self):
        good = {"passed": True, "final_table_absent": True, "mode": "configuration",
                "settings": [1, 0, 1], "denied": ["temp_table", "qualified_temp_table", "temp_view", "temp_trigger", "attach"],
                "rollback_restored": True, "trusted_schema": "unqualified", "production_admission": False}
        self.assertTrue(controller.result_ok(200, good, True))
        self.assertFalse(controller.result_ok(500, good, True))
        self.assertFalse(controller.result_ok(200, {"passed": True, "final_table_absent": True}, True))
        for key in good:
            changed = dict(good)
            del changed[key]
            self.assertFalse(controller.result_ok(200, changed, True), key)
        for key, value in [("settings", [0, 0, 1]), ("denied", good["denied"][:-1]),
                           ("production_admission", True), ("trusted_schema", "qualified")]:
            self.assertFalse(controller.result_ok(200, dict(good, **{key: value}), True))

    def test_response_bound_and_json(self):
        self.assertEqual(controller.bounded_json(io.BytesIO(b'{"passed":false}')), {"passed": False})
        with self.assertRaises(RuntimeError):
            controller.bounded_json(io.BytesIO(b" " * 8193))
        with self.assertRaises(json.JSONDecodeError):
            controller.bounded_json(io.BytesIO(b"not JSON"))


if __name__ == "__main__":
    unittest.main()
