# Third-party code and data

Vision Overlay's original code is distributed under Apache-2.0. This does not
relicense third-party code, game data, names, artwork or other resources.

| Component | Source / revision | Notice |
|---|---|---|
| auto-artifactarium, patched | konkers/auto-artifactarium `7dafbf0af0bfece23e46382dd3f991d39a5436ac` | MIT; `vendor/auto-artifactarium/LICENSE` and patch notes |
| pktmon 0.6.2, patched | emmachase/pktmon | MIT; `vendor/pktmon/LICENSE`, `VISION_PATCHES.md` |
| Shared dispatch data | konkers/irminsul `49e66bdfc43c4c6e4293e7f1b7e85b6988069ae4`, `keys/gi.json` | Upstream MIT notice retained in `packaging/dispatch/Irminsul-LICENSE.txt`; exact source/hash in `source.json` |
| Banner metadata | Made Baruna / paimon-moe `4b873a6decad0384665084c69e9927a9fa266555` | MIT; `web/src/data/PAIMON_LICENSE.txt`; extracted as data, without executing upstream JavaScript |
| Nunito | The Nunito Project Authors | SIL Open Font License 1.1; `web/public/fonts/Nunito-OFL.txt` |
| React, React DOM, scheduler | npm packages pinned in `web/package-lock.json` | MIT notices copied into the portable distribution |
| Python 3.14.10 embeddable | python.org official Windows x64 package | Python's full runtime license is included in `components/python/LICENSE.txt`; URL/SHA256 pinned in the build script |
| Rust dependencies | `Cargo.lock` | Package SPDX declarations and supplied notices are collected into `licenses/rust-dependencies.json` and `licenses/rust/` |

Some published Rust crates omit standalone license files. Pinned upstream
notices, declared license information and attribution are retained under
`packaging/license-overrides/`; `sources.json` records downloaded notice hashes.
For dependencies offering MIT as one alternative, the MIT alternative is used
by those overrides. The build fails if a dependency has no supplied or reviewed
notice rather than silently leaving an empty entry.

The two PEM files in the vendored decoder and the shared dispatch JSON are
unchanged public upstream decryption material, **not publisher credentials or
user account secrets**. `packaging/public-crypto-material.json` records exact
hashes and immutable source URLs. Publication checks allow only these precise
files; they do not exempt other keys, tokens or private data.

## Game reference data and images

The pinned bootstrap contains derived game reference tables and Russian text
from `https://gitlab.com/Dimbreath/animegamedata2`, revision recorded in
`packaging/catalog/source.json`, including hashes of the input source files.
It contains no account snapshots, profiles, tokens, game executables or game
metadata binaries. Source attribution is not a grant of ownership over the
underlying game content: game names, text, artwork and trademarks remain the
property of their respective owners, including HoYoverse.

Artwork is fetched at runtime from Enka.Network and PathOfGenshin/resources;
it is not included in the source release or initial portable archive. Fetched
images are cached locally. Vision Overlay is an unofficial companion and is
not affiliated with or endorsed by HoYoverse.
