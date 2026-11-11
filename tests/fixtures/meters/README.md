# Meter fixtures

Real USB captures are **not** committed to this repository: they may contain
private or licensed content, and they are large binary blobs.

`frames.jsonl` holds eight **sanitized synthetic frames**. Each one is built
from the measured layout in `docs/METER_PROTOCOL.md`: the 34 channels carry
current/peak values, six of the measured frames reproduce the mic and stream
channels seen in the captures, and `all-groups` pins one distinct level on
every channel of every group. Every other field is the constant `15114`.
The frames contain only those numbers, no capture data. The expected values
live in `tests/test-decode-meters.py`.

The schema follows `tools/normalize-usbmon.py`:

```json
{"endpoint":"0x83","transfer":"bulk","direction":"in","data":"04f0...","time":1.0}
```

`tools/decode-meters.py` consumes that JSON Lines schema (one JSON object per
line) produced by the normalizer.
