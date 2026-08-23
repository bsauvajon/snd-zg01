#!/usr/bin/env python3
"""Convert ZG01 USB control-transfer TSV rows into stable JSON Lines."""

import argparse
import csv
import io
import json
from pathlib import Path

ZG01_VENDOR = 0x0499
ZG01_PRODUCT = 0x1513


def integer(value: str) -> int:
    return int(value, 0)


def normalize(path: Path):
    raw = path.read_bytes()
    encoding = "utf-16" if raw.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig"
    with io.StringIO(raw.decode(encoding), newline="") as stream:
        for line_number, row in enumerate(csv.reader(stream, delimiter="\t"), 1):
            if not row or all(not field for field in row):
                continue
            if len(row) != 11:
                raise ValueError(f"line {line_number}: expected 11 TSV fields, got {len(row)}")

            frame, timestamp, device, vendor, product, request_type, request, value, index, length, data = row
            if integer(vendor) != ZG01_VENDOR or integer(product) != ZG01_PRODUCT:
                continue

            request_type_value = integer(request_type)
            compact_data = data.replace(":", "").replace(" ", "").lower()
            try:
                payload = bytes.fromhex(compact_data)
            except ValueError as error:
                raise ValueError(f"line {line_number}: capdata is not hexadecimal") from error
            expected_length = integer(length)
            actual_length = len(payload)
            if actual_length > expected_length:
                raise ValueError(
                    f"line {line_number}: wLength is {expected_length}, capdata is {actual_length} bytes"
                )
            yield {
                "frame": integer(frame),
                "time": float(timestamp),
                "device": device,
                "direction": "in" if request_type_value & 0x80 else "out",
                "request_type": request_type_value,
                "request": integer(request),
                "value": integer(value),
                "index": integer(index),
                "length": expected_length,
                "actual_length": actual_length,
                "data": compact_data,
            }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture_tsv", type=Path)
    args = parser.parse_args()
    for transfer in normalize(args.capture_tsv):
        print(json.dumps(transfer, separators=(",", ":"), sort_keys=True))


if __name__ == "__main__":
    main()
