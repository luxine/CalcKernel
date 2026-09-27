"""Bind original-process raw observations to the unmodified schema-7 report."""

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/check-runtime-observations.py"
CHANNELS = ("candidate", "cSimd", "rustSimd")
FIELDS = ("wallNs", "threadCpuNs", "cpu", "userCpuNs", "systemCpuNs",
          "minorFaults", "majorFaults", "voluntarySwitches", "involuntarySwitches")


class RuntimeObservationTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "original-process evidence checker is missing")
        spec = importlib.util.spec_from_file_location("runtime_observation", SCRIPT)
        self.checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.checker)

    def fixture(self, root):
        directory = root / "measurement-123-456"
        directory.mkdir()
        artifacts, libraries = [], []
        for channel in CHANNELS:
            name = f"vector-specialized_length-checked-{channel}.so"
            data = channel.encode()
            (directory / name).write_bytes(data)
            library = dict(file=name, bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
            libraries.append(library)
            artifacts.append(dict(suite="vector", case="specialized_length", mode="checked",
                                  channel=channel, **library))
        values = [((i + 7) * 2654435761 & 0xffffffff) % 1000002 + 1 for i in range(4000)]
        digest = lambda values: hashlib.sha256(b"".join(v.to_bytes(4, "little") for v in values)).hexdigest()
        identity = dict(type="identity", schemaVersion=1, acceptance=False,
                        case="specialized_length", mode="checked", pid=123,
                        warmup=3, iterations=20, repetitions=7, calls=5000, elements=20000000,
                        inputAddress=100, outputAddress=200, entries=[300, 400, 500],
                        inputSha256=digest(values), resultDigest=digest([v + 13 for v in values]),
                        libraries=libraries, maps=None, kernel=None, cpuinfo=None)
        rows = [identity]
        for warmup, rounds, repetitions in ((True, 3, 1), (False, 20, 7)):
            for round_index in range(rounds):
                for repetition in range(repetitions):
                    for offset in range(3):
                        channel = (round_index + repetition + offset) % 3
                        rows.append(dict(type="sample", sequence=len(rows) - 1,
                                         channel=channel, warmup=warmup,
                                         gateNs=1000 + channel * 100 + round_index + repetition,
                                         before=dict.fromkeys(FIELDS), after=dict.fromkeys(FIELDS)))
        rows.append(dict(type="complete", rows=429, samplingSucceeded=True, overflow=False))
        case = dict(name="specialized_length", batchIterations=20000000,
                    resultDigest=identity["resultDigest"],
                    warmupOrder=[[(i + j) % 3 for j in range(3)] for i in range(3)],
                    sampleOrder=[[(i + j) % 3 for j in range(3)] for i in range(20)])
        for index, channel in enumerate(CHANNELS):
            case[channel + "SamplesNs"] = [1003 + index * 100 + i for i in range(20)]
            case[channel + "MedianNs"] = 1013 + index * 100
        report = dict(schemaVersion=7, evidenceDirectory=directory.name, oracleArtifacts=artifacts,
                      vectorSuites=[dict(mode="checked", cases=[case])])
        path = root / "results.json"
        path.write_text(json.dumps(report))
        raw = directory / "checked-runtime-observations.jsonl"
        raw.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
        return path, raw, report, rows

    def test_reconstructs_all_report_medians_with_unavailable_metrics(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, _, _, _ = self.fixture(Path(temporary))
            summary = self.checker.analyze(path)
            self.assertFalse(summary["acceptance"])
            self.assertEqual(summary["rawRows"], 429)
            self.assertEqual(summary["channels"][0]["medianNs"], 1013)
            self.assertEqual(summary["channels"][0]["availableDeltas"]["systemCpuNs"], 0)

    def test_rejects_raw_corruption_and_report_drift(self):
        mutations = ("missing", "duplicate", "order", "warmup", "duration", "identity",
                     "footer", "counter", "median", "library", "duplicate_key")
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                path, raw, report, original = self.fixture(Path(temporary))
                rows = copy.deepcopy(original)
                if mutation == "missing":
                    rows.pop(10)
                elif mutation == "duplicate":
                    rows.insert(10, rows[10])
                elif mutation == "order":
                    rows[10], rows[11] = rows[11], rows[10]
                elif mutation == "warmup":
                    rows[10]["warmup"] = True
                elif mutation == "duration":
                    rows[10]["gateNs"] = 0
                elif mutation == "identity":
                    rows[0]["inputSha256"] = "0" * 64
                elif mutation == "footer":
                    rows[-1]["samplingSucceeded"] = False
                elif mutation == "counter":
                    rows[10]["before"]["systemCpuNs"] = -1
                elif mutation == "median":
                    report["vectorSuites"][0]["cases"][0]["candidateSamplesNs"][0] += 1
                elif mutation == "library":
                    (raw.parent / rows[0]["libraries"][0]["file"]).write_bytes(b"changed")
                path.write_text(json.dumps(report))
                text = "\n".join(json.dumps(row) for row in rows) + "\n"
                if mutation == "duplicate_key":
                    text = text.replace('"gateNs": 1000', '"gateNs": 1000, "gateNs": 1000', 1)
                raw.write_text(text)
                with self.assertRaises(ValueError):
                    self.checker.analyze(path)

    def test_reports_resource_events_and_cpu_changes_without_causal_verdict(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, raw, _, rows = self.fixture(Path(temporary))
            rows[10]["before"].update(systemCpuNs=1000, involuntarySwitches=1, cpu=2)
            rows[10]["after"].update(systemCpuNs=2000, involuntarySwitches=2, cpu=3)
            raw.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
            summary = self.checker.analyze(path)
            channel = summary["channels"][0]
            self.assertEqual(channel["availableDeltas"]["systemCpuNs"], 1)
            self.assertEqual(channel["positiveDeltas"]["systemCpuNs"], 1)
            self.assertEqual(summary["observedCpus"], [2, 3])
            self.assertFalse(summary["acceptance"])
