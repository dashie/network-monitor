"""Tests of the Uptime Kuma converter, runnable without a test framework.

    python3 -m unittest discover -s scripts/utils -p 'test_*.py'
"""

import tempfile
import unittest
from pathlib import Path

import kuma2nm


def heartbeat(monitor, status, time, message="", ping=""):
    return {
        "id": "1",
        "monitor_name": monitor,
        "status": status,
        "time": time,
        "message": message,
        "ping": ping,
    }


class AddressTest(unittest.TestCase):
    def test_the_address_follows_the_ping_command(self):
        self.assertEqual(
            kuma2nm.address_of("PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data."),
            "1.1.1.1",
        )

    def test_an_icmp_error_names_the_target_and_not_the_router(self):
        message = (
            "PING 192.168.1.1 (192.168.1.1) 56(84) bytes of data. "
            "From 192.168.2.1 icmp_seq=1 Destination Net Unreachable"
        )
        self.assertEqual(kuma2nm.address_of(message), "192.168.1.1")

    def test_a_message_without_a_ping_names_no_address(self):
        self.assertIsNone(kuma2nm.address_of(""))
        self.assertIsNone(kuma2nm.address_of("monitor is paused"))


class FieldTest(unittest.TestCase):
    def test_the_time_becomes_an_instant_in_utc(self):
        self.assertEqual(
            kuma2nm.instant("2026-07-05 16:23:47.590"), "2026-07-05T16:23:47.590Z"
        )
        self.assertEqual(kuma2nm.instant("2026-07-05 16:23:47"), "2026-07-05T16:23:47.000Z")

    def test_the_ping_becomes_microseconds(self):
        self.assertEqual(kuma2nm.micros("6.56"), "6560")
        self.assertEqual(kuma2nm.micros("6"), "6000")
        self.assertEqual(kuma2nm.micros(""), "")

    def test_an_unreadable_ping_is_refused(self):
        with self.assertRaises(ValueError):
            kuma2nm.micros("n/a")


class ConvertTest(unittest.TestCase):
    def convert(self, rows, **options):
        return kuma2nm.convert(
            rows,
            options.get("initial", {}),
            options.get("aliases", {}),
            options.get("labels", {}),
            options.get("skipped", set()),
        )

    def test_a_down_heartbeat_states_the_target_it_names(self):
        rows = [
            heartbeat(
                "DNS", "0", "2026-07-05 16:23:47.590", "PING 1.1.1.1 (1.1.1.1) ..."
            )
        ]
        output, counts, per_target = self.convert(rows, labels={"1.1.1.1": "dns"})
        self.assertEqual(
            [line for _, _, line in output],
            ["1.1.1.1,dns,,down,2026-07-05T16:23:47.590Z,"],
        )
        self.assertEqual(counts["kept"], 1)
        self.assertEqual(per_target, {"1.1.1.1": 1})

    def test_an_up_heartbeat_keeps_the_address_of_its_monitor(self):
        rows = [
            heartbeat("DNS", "0", "2026-07-05 16:23:47.590", "PING 1.1.1.1 ..."),
            heartbeat("DNS", "1", "2026-07-05 16:24:02.590", ping="6.56"),
        ]
        output, _, _ = self.convert(rows)
        self.assertEqual(
            [line for _, _, line in output],
            [
                "1.1.1.1,,,down,2026-07-05T16:23:47.590Z,",
                "1.1.1.1,,,up,2026-07-05T16:24:02.590Z,6560",
            ],
        )

    def test_a_repointed_monitor_splits_into_two_targets(self):
        rows = [
            heartbeat("DNS", "0", "2026-07-05 16:23:47.590", "PING 1.1.1.1 ..."),
            heartbeat("DNS", "0", "2026-08-17 05:21:03.875", "PING 8.8.8.8 ..."),
            heartbeat("DNS", "1", "2026-08-17 05:21:08.000", ping="7.1"),
        ]
        output, _, per_target = self.convert(rows)
        self.assertEqual(
            [line.split(",")[0] for _, _, line in output],
            ["1.1.1.1", "8.8.8.8", "8.8.8.8"],
        )
        self.assertEqual(per_target, {"1.1.1.1": 1, "8.8.8.8": 2})

    def test_an_alias_records_a_measured_address_under_another_target(self):
        rows = [heartbeat("DNS", "0", "2026-08-17 05:21:03.875", "PING 8.8.8.8 ...")]
        output, _, _ = self.convert(rows, aliases={"8.8.8.8": "1.1.1.1"})
        self.assertEqual(
            [line for _, _, line in output],
            ["1.1.1.1,,,down,2026-08-17T05:21:03.875Z,"],
        )

    def test_an_initial_address_covers_the_first_heartbeats(self):
        rows = [heartbeat("DNS", "1", "2026-07-05 16:24:02.590", ping="6.56")]
        output, _, _ = self.convert(rows, initial={"DNS": "1.1.1.1"})
        self.assertEqual([line.split(",")[0] for _, _, line in output], ["1.1.1.1"])

    def test_a_monitor_without_an_address_is_refused(self):
        rows = [heartbeat("DNS", "1", "2026-07-05 16:24:02.590", ping="6.56")]
        with self.assertRaises(LookupError):
            self.convert(rows)

    def test_another_status_is_skipped_and_counted(self):
        rows = [
            heartbeat("DNS", "2", "2026-07-05 16:23:47.590", "PING 1.1.1.1 ..."),
            heartbeat("DNS", "3", "2026-07-05 16:23:52.590"),
            heartbeat("DNS", "1", "2026-07-05 16:24:02.590", ping="6.56"),
        ]
        output, counts, _ = self.convert(rows)
        self.assertEqual(len(output), 1)
        self.assertEqual(counts["skipped_status"], 2)

    def test_a_skipped_address_is_left_out(self):
        rows = [
            heartbeat("DNS", "0", "2026-08-17 05:21:03.875", "PING 8.8.8.8 ..."),
            heartbeat("GW", "0", "2026-08-17 05:21:04.875", "PING 192.168.1.1 ..."),
        ]
        output, counts, _ = self.convert(rows, skipped={"8.8.8.8"})
        self.assertEqual([line.split(",")[0] for _, _, line in output], ["192.168.1.1"])
        self.assertEqual(counts["skipped_address"], 1)


EXPECTED = """target,label,interface,state,occurred_at,rtt_us
1.1.1.1,cloudflare-dns,,down,2026-09-01T10:00:00.000Z,
1.1.1.1,cloudflare-dns,,up,2026-09-01T10:00:20.000Z,6560
1.1.1.1,cloudflare-dns,,down,2026-09-02T08:00:00.000Z,
1.1.1.1,cloudflare-dns,,up,2026-09-02T08:00:20.000Z,7400
1.1.1.1,cloudflare-dns,,up,2026-09-02T08:00:40.000Z,7200
192.168.1.1,gateway,,down,2026-09-01T10:05:00.000Z,
192.168.1.1,gateway,,up,2026-09-01T10:05:20.000Z,1120
"""


class ExampleTest(unittest.TestCase):
    """The example the runbook states must stay the output of the command it shows."""

    def test_the_shipped_example_converts_to_the_documented_output(self):
        origin = Path(__file__).with_name("kuma-example.csv")
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "events.csv"
            self.assertEqual(
                kuma2nm.main(
                    [
                        str(origin),
                        "--initial",
                        "DNS ping=1.1.1.1",
                        "--initial",
                        "Gateway ping=192.168.1.1",
                        "--alias",
                        "8.8.8.8=1.1.1.1",
                        "--label",
                        "1.1.1.1=cloudflare-dns",
                        "--label",
                        "192.168.1.1=gateway",
                        "-o",
                        str(target),
                    ]
                ),
                0,
            )
            self.assertEqual(target.read_text(encoding="utf-8"), EXPECTED)


class CommandTest(unittest.TestCase):
    def test_the_file_is_written_in_target_then_instant_order(self):
        source = (
            "id,monitor_name,status,time,message,ping\n"
            '2,GW,0,2026-08-17 05:21:04.875,PING 192.168.1.1 ...,\n'
            '1,DNS,0,2026-08-17 05:21:03.875,PING 1.1.1.1 ...,\n'
        )
        with tempfile.TemporaryDirectory() as directory:
            origin = Path(directory) / "kuma.csv"
            origin.write_text(source, encoding="utf-8")
            target = Path(directory) / "events.csv"
            self.assertEqual(
                kuma2nm.main([str(origin), "-o", str(target)]), 0
            )
            self.assertEqual(
                target.read_text(encoding="utf-8"),
                "target,label,interface,state,occurred_at,rtt_us\n"
                "1.1.1.1,,,down,2026-08-17T05:21:03.875Z,\n"
                "192.168.1.1,,,down,2026-08-17T05:21:04.875Z,\n",
            )

    def test_an_unreadable_file_exits_with_two(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "absent.csv"
            self.assertEqual(kuma2nm.main([str(missing)]), 2)


if __name__ == "__main__":
    unittest.main()
