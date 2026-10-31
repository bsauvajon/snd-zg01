# Meter fixtures

Real USB captures are **not** committed to this repository: they may contain
private or licensed content, and they are large binary blobs.

Any fixture used by the meter tests is a **sanitized synthetic frame** built
in the test script itself, following the schema produced by
`tools/normalize-usbmon.py`:

```json
{"endpoint":"0x83","transfer":"bulk","direction":"in","data":"04f04310043e1402...","time":1.0}
```

`tools/decode-meters.py` consumes that JSON Lines schema (one JSON object per
line) produced by the normalizer.
