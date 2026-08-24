# Cross-check corpora

These four fixture sets are not ours. They come from
[zkasuran/telegraph-salience-scorer](https://github.com/zkasuran/telegraph-salience-scorer)
at commit `0174a85`, MIT licensed, copyright 2026 zkasuran. That repository is the
author of the module currently holding the `URL_SCAN` champion slot.

They are vendored here because Telegraph's own benchmark is not public and a scorer
tuned only against fixtures its own author wrote will overfit them. `benchmark.json`
spans 20 canonical intents; the three `family-*.json` sets cover figures, authenticity
verdicts and named entities. VerdictLock is measured against all four on every run,
alongside the champion binary itself.

Nothing in this directory is used at runtime. Delete it and the module still builds.
