"""ModelTrace 离线适配，模型调用仍由宿主管理。"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parents[1]))
VENDOR = ROOT / "vendor/modeltrace"
sys.path.insert(0, str(VENDOR))

import fingerprint
from gpt56_vnext.errors import AppError

SOURCE = json.loads((VENDOR / "SOURCE.json").read_text())
BANK_PATH = VENDOR / "data/unified_bank.json"
BANK = fingerprint.load_bank(BANK_PATH)
BANK_HASH = hashlib.sha256(BANK_PATH.read_bytes()).hexdigest()
LEGACY_BANK_PATH = VENDOR / "data/unified_bank-d4131b3.json"
LEGACY_BANK_HASH = hashlib.sha256(LEGACY_BANK_PATH.read_bytes()).hexdigest()
BANKS = {
    (BANK_HASH, SOURCE["commit"][:7]): BANK,
    (LEGACY_BANK_HASH, "d4131b3"): fingerprint.load_bank(LEGACY_BANK_PATH),
}
MAX_NUMBERS = 4096


def catalog():
    return {"method": "modeltrace", "label": "ModelTrace", "id": "modeltrace-unified",
            "engine_version": SOURCE["commit"][:7], "version": SOURCE["commit"][:7],
            "hash": BANK_HASH, "models": [{"id": m["id"], "label": m["display_name"]} for m in BANK["models"]],
            "family_count": len({m["family"] for m in BANK["models"]}),
            "tiers": [{"id": "fixed", "count": 3}]}


def check_plan(plan):
    bank = BANKS.get((plan["hash"], plan["engine_version"]))
    if bank is None:
        raise AppError("baseline_changed")
    if plan["tier"] != "fixed" or plan["claimed_model"] != "unified":
        raise AppError("invalid_modeltrace_plan")
    jobs = plan["jobs"]
    if len(jobs) != 3 or len({j["job_id"] for j in jobs}) != 3:
        raise AppError("invalid_modeltrace_plan")
    for job in jobs:
        if not isinstance(job["expected_count"], int) or not 292 <= job["expected_count"] <= 332:
            raise AppError("invalid_modeltrace_plan")
        if not isinstance(job["prompt"], str) or not 1 <= len(job["prompt"].encode()) <= 4096:
            raise AppError("invalid_modeltrace_plan")
    return bank


def prepare(data):
    if data["tier"] != "fixed" or data["claimed_model"] != "unified":
        raise AppError("invalid_modeltrace_plan")
    if not isinstance(data["request_model"], str) or not 1 <= len(data["request_model"].strip()) <= 256:
        raise AppError("invalid_request_model")
    meta = catalog()
    # 随机题目只生成一次，重开进程和失败续测均复用已持久化的题目。
    return {k: v for k, v in {
        **meta, "tier": "fixed", "claimed_model": "unified", "request_model": data["request_model"],
        "jobs": [{"job_id": c["id"], "prompt": c["prompt"], "expected_count": c["expected_count"]}
                 for c in fingerprint.generate_challenges(3)], "retry_budget": 2,
    }.items() if k not in {"models", "tiers", "family_count", "label"}}


def evaluate(answer, job, bank=None):
    bank = BANK if bank is None else bank
    numbers = fingerprint.parse_numbers(answer)
    if len(numbers) > MAX_NUMBERS:
        return {"status": "error", "error": "too_many_numbers"}
    try:
        analysis = fingerprint.analyze_global_outputs(
            [{"text": answer, "expected_count": job["expected_count"]}], bank)
    except ValueError as error:
        # 只把上游明确的短答错误转换为样本失败，其他评分故障保持可见。
        if str(error) != "没有可用回答：请粘贴完整数字序列；拒答或严重截断的回答不会计入。":
            raise
        return {"status": "error", "error": "insufficient_numbers", "parsed_numbers": len(numbers)}
    # 保存完整有序数字而非回答全文，既保留原评分输入，也限制宿主状态体积。
    return {"status": "ok", "error": None, "numbers": numbers,
            "parsed_numbers": len(numbers), "minimum_numbers": analysis["diagnostics"][0]["minimum_numbers"]}


def score(plan, results):
    bank = check_plan(plan)
    jobs = {j["job_id"]: j for j in plan["jobs"]}
    outputs, seen = [], set()
    for result in results:
        if result["status"] != "ok":
            continue
        identity = result["job_id"]
        if identity not in jobs or identity in seen:
            raise AppError("duplicate_or_unknown_sample")
        seen.add(identity)
        numbers = result["numbers"]
        if not isinstance(numbers, list) or len(numbers) > MAX_NUMBERS or any(
                type(n) is not int or not 1 <= n <= 355 for n in numbers):
            raise AppError("invalid_number_sample")
        outputs.append({"text": ",".join(map(str, numbers)), "expected_count": jobs[identity]["expected_count"]})
    analysis = fingerprint.analyze_global_outputs(outputs, bank) if outputs else None
    valid = analysis["used_outputs"] if analysis else 0
    complete = valid == len(jobs)
    return {"method": "modeltrace", "verdict": "attributed" if complete else "insufficient",
            "valid_samples": valid, "planned_samples": len(jobs),
            "reasons": [] if complete else ["samples_incomplete" if valid else "no_valid_samples"],
            "analysis": analysis}
