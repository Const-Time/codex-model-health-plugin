import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import worker
import modeltrace_adapter as adapter


def sse(text):
    return 'data: ' + json.dumps({"type": "response.completed", "response": {
        "id": "fixture", "status": "completed", "output": [{"type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]}]}}) + '\n\n'


class ModelTraceTests(unittest.TestCase):
    def setUp(self):
        self.plan = worker.handle({"op": "prepare", "method": "modeltrace", "tier": "fixed",
                                   "claimed_model": "unified", "request_model": "gpt-6-astra"})

    def sample(self, job, text):
        return {"job_id": job["job_id"], **worker.handle({"op": "evaluate", "plan": self.plan,
                "job_id": job["job_id"], "sse": sse(text)})}

    def score(self, samples):
        return worker.handle({"op": "score", "plan": self.plan, "results": samples})

    def test_vendor_is_exact_and_catalog_has_two_methods(self):
        for name, digest in adapter.SOURCE["files"].items():
            self.assertEqual(hashlib.sha256((adapter.VENDOR / name).read_bytes()).hexdigest(), digest)
        catalogs = worker.handle({"op": "catalog"})["methods"]
        self.assertEqual([c["method"] for c in catalogs], ["meow", "modeltrace"])
        self.assertEqual(len(catalogs[1]["models"]), 18)
        self.assertEqual(catalogs[1]["family_count"], 2)

    def test_legacy_bank_remains_available_for_saved_runs(self):
        plan = {**self.plan, 'hash':adapter.LEGACY_BANK_HASH, 'engine_version':'d4131b3', 'version':'d4131b3'}
        bank = adapter.check_plan(plan)
        self.assertEqual(len(bank['models']), 17)
        samples = [{'job_id':job['job_id'], **adapter.evaluate('7,' * 333, job, bank)} for job in plan['jobs']]
        result = adapter.score(plan, samples)
        self.assertEqual(result['valid_samples'], 3)
        self.assertEqual(len(result['analysis']['results']), 17)

    def test_frozen_challenges_and_payload_use_provider_defaults(self):
        restored = json.loads(json.dumps(self.plan))
        self.assertEqual(len(restored["jobs"]), 3)
        self.assertEqual(restored["retry_budget"], 2)
        for job in restored["jobs"]:
            self.assertIn(str(job["expected_count"]), job["prompt"])
            query = {"op": "payload", "plan": restored, "job_id": job["job_id"]}
            payload = worker.handle(query)
            self.assertEqual(payload, {"model": "gpt-6-astra", "stream": True, "store": False,
                                      "input": [{"role": "user", "content": job["prompt"]}]})
            self.assertEqual(worker.handle(query), payload)

    def test_exact_original_score_for_one_two_three_answers(self):
        samples, outputs = [], []
        for index, job in enumerate(self.plan["jobs"]):
            text = ', '.join(str(1 + (i * (37 + index) + 13) % 355) for i in range(job["expected_count"]))
            samples.append(self.sample(job, text))
            outputs.append({"text": text, "expected_count": job["expected_count"]})
            report = self.score(samples)
            self.assertEqual(report["analysis"], adapter.fingerprint.analyze_global_outputs(outputs, adapter.BANK))
            self.assertEqual(report["valid_samples"], index + 1)
            self.assertEqual(report["verdict"], "attributed" if index == 2 else "insufficient")
            self.assertAlmostEqual(sum(r["probability"] for r in report["analysis"]["results"]), 1)
            self.assertTrue(samples[-1]["answer_truncated"])
            self.assertLessEqual(len(samples[-1]["answer"].encode()), 256)
        self.assertLess(len(json.dumps({"plan": self.plan, "attempts": samples, "report": report}).encode()), 262144)

    def test_length_threshold_and_preserved_order(self):
        import math
        job = self.plan["jobs"][0]
        minimum = max(80, math.ceil(job["expected_count"] * .55))
        short = self.sample(job, '7,' * (minimum - 1))
        self.assertEqual(short["error"], "insufficient_numbers")
        accepted = self.sample(job, '7,' * minimum)
        self.assertEqual(accepted["status"], "ok")
        self.assertEqual(accepted["numbers"], [7] * minimum)
        self.assertEqual(accepted["minimum_numbers"], minimum)
        answer = '说明 2 项文字\n' + ','.join(str(i % 355 + 1) for i in range(333))
        self.assertEqual(self.sample(job, answer)["numbers"], adapter.fingerprint.parse_numbers(answer))

    def test_error_refusal_truncation_and_no_samples_do_not_attribute(self):
        job = self.plan["jobs"][0]
        for stream in ['data: {"type":"response.refusal.done"}\n\n',
                       'data: {"type":"response.output_text.delta","delta":"1,2"}\n\n',
                       'data: {"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}\n\n']:
            result = worker.handle({"op": "evaluate", "plan": self.plan, "job_id": job["job_id"], "sse": stream})
            self.assertEqual(result["status"], "error")
        no_answer = self.sample(job, 'I will not produce a sequence')
        self.assertEqual(no_answer["status"], "error")
        self.assertEqual(self.score([no_answer]), self.score([]))
        self.assertIsNone(self.score([])["analysis"])

    def test_duplicate_unknown_stale_and_corrupt_samples_rejected(self):
        job = self.plan["jobs"][0]
        result = self.sample(job, '7,' * 333)
        for samples in [[result, result], [{**result, "job_id": "missing"}], [{**result, "numbers": [True] * 333}]]:
            with self.assertRaises(worker.AppError):
                self.score(samples)
        for field in ['hash', 'engine_version']:
            altered = copy.deepcopy(self.plan)
            altered[field] = 'changed'
            with self.assertRaises(worker.AppError):
                worker.handle({"op": "score", "plan": altered, "results": []})
        self.assertEqual(self.sample(job, '7,' * 4097)["error"], "too_many_numbers")

    def test_legacy_meow_has_unchanged_results(self):
        plan = worker.prepare({"tier": "low", "claimed_model": "gpt-6-astra", "request_model": "gpt-6-astra"})
        del plan["method"]
        self.assertEqual(worker.handle({"op": "score", "plan": plan, "results": []}), worker.score(plan, []))
        for method in ['unknown', None]:
            with self.assertRaises(worker.AppError):
                worker.handle({"op": "prepare", "method": method})

    def test_scoring_fault_is_not_disguised_as_a_short_answer(self):
        from unittest.mock import patch
        with patch.object(adapter.fingerprint, 'analyze_global_outputs', side_effect=ValueError('broken bank')):
            with self.assertRaisesRegex(ValueError, 'broken bank'):
                self.sample(self.plan["jobs"][0], '7,' * 333)


if __name__ == '__main__':
    unittest.main()
