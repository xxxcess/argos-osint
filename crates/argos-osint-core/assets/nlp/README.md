# Pinned NLP assets

Compromise 14.18.0 and dates 3.9.0, copied byte-for-byte from upstream commit `2e7a1b958347c1dc47858e7ded6b7efce5662c0e`.

| Local asset | Upstream path | Bytes | SHA-256 |
| --- | --- | --- | --- |
| compromise.js | builds/compromise.js | 350773 | 61e2541597f27c592947e5059c083548f24aa6fba14cf2b109a00db44de6ca57 |
| compromise-dates.min.js | plugins/dates/builds/compromise-dates.min.js | 124675 | 3bfc87cf5c0fc6d4190c195078aa969675e9de32a70836cb233d87b724d6aaf2 |

The upstream MIT license is retained in LICENSE. Runtime checks both bundle hashes before initialization; neither file receives an added newline. These bundles are embedded only by the optional `nlp` feature.
