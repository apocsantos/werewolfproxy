"""Executable checks for the frozen Fang lifecycle characterization."""
import json
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangRegistryCharacterization(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = ROOT / "tests" / "fixtures" / "fang_registry_characterization.json"
        cls.data = json.loads(path.read_text())

    def test_runtime_fields_are_distinct_from_profiles(self):
        self.assertEqual(self.data["runtime_state"], ["fangs", "fang_tasks", "fang_started"])
        self.assertTrue(self.data["preserved_characteristics"]["active_profiles_are_persistent_names"])

    def test_current_lifecycle_outcomes_are_frozen(self):
        characteristics = self.data["preserved_characteristics"]
        self.assertEqual(characteristics["duplicate_local_address"], "FANG_ALREADY_ACTIVE")
        self.assertTrue(characteristics["close_removes_record_and_tracked_task"])
        self.assertTrue(characteristics["cleanup_removes_finished_listener_tasks"])
        self.assertTrue(characteristics["restore_generates_new_ids"])

    def test_known_incomplete_shutdown_semantics_are_explicit(self):
        characteristics = self.data["preserved_characteristics"]
        self.assertTrue(characteristics["pack_revoke_does_not_guarantee_child_task_shutdown"])
        self.assertTrue(characteristics["silver_clear_does_not_guarantee_detached_child_shutdown"])


if __name__ == "__main__":
    unittest.main()
