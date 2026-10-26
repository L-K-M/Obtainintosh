# Vendored license texts

License texts for bundled components whose packages do not publish them.
The notice generators read them through the override files next to them;
each file is verbatim from the upstream reference below (fetched 2026-10-26),
except where noted.

| File | Upstream reference | SHA-256 |
| --- | --- | --- |
| `fonts/Sysfont-OFL.txt` | SIL OFL 1.1 from openfontlicense.org/documents/OFL.txt, headed by the copyright from the font's name table ("© Alina Sava [2021]") | `662a3ee09e53760c3cbee897b0d6a9c086ee257ee3a79e78b46860d0f294a6a7` |
| `fonts/Geneva-9.1-OFL.txt` | SIL OFL 1.1 as above, headed by the copyright from the font's name table ("Copyright Kelsey Higham 2016") | `f6bc9666c787a79415f1d7a803ff3329ab98e5d8b8771d883fbdffb645e6f819` |
| `npm/tauri-plugin-opener-2.5.5-LICENSE_MIT` | tauri-apps/plugins-workspace `plugins/opener/LICENSE_MIT` at `opener-js-v2.5.5` | `89ff9689dcf9dd53968785d05a26f7898bb169dbfcada8d032b3e68cf0d55607` |
| `npm/tauri-plugin-opener-2.5.5-LICENSE_APACHE-2.0` | tauri-apps/plugins-workspace `plugins/opener/LICENSE_APACHE-2.0` at `opener-js-v2.5.5` | `0cec06e0e55fbc3dc5cee4fca9b607f66cb8f4e4dbcf3b3c013594dd156732e9` |
| `rust/alloc-no-stdlib-0.2.4-LICENSE` | dropbox/rust-alloc-no-stdlib `LICENSE` at `0.2.4` | `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae` |
| `rust/dlopen2-LICENSE` | OpenByteDev/dlopen2 `LICENSE` (last changed in `d0304f3`, 2022) | `39fa265207450e77c62e90c5594a06c085b655d8374c7ced4bf7894b6bd95dd2` |
| `rust/objc2-LICENSE-MIT.txt` | madsmtm/objc2 `LICENSE-MIT.txt` (added in `ee9a7ad`, 2026) | `f6a2e783669a0dd7a2c3b8a4f76db40ae49033e66e1bdc30f3aa25fe5577b070` |
| `rust/rust-unic-0.9.0-LICENSE-MIT` | open-i18n/rust-unic `LICENSE-MIT` at `v0.9.0` | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| `rust/rust-unic-0.9.0-LICENSE-APACHE` | open-i18n/rust-unic `LICENSE-APACHE` at `v0.9.0` | `a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2` |
| `rust/siphasher-1.0.4-LICENSE-MIT` | jedisct1/rust-siphash `LICENSE-MIT` at `1.0.4` | `f9c4c77baa3828004ee54b8a4f2db2e88ed44a6237a493965bf551fac0fcb62d` |
| `rust/siphasher-1.0.4-LICENSE-APACHE` | jedisct1/rust-siphash `LICENSE-APACHE` at `1.0.4` | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| `linux/appimage/type2-runtime-LICENSE.txt` | AppImage/type2-runtime `LICENSE` at `75849dce7cc37e4319b633df1f116ca895c71a12` | `aa154fc9070614bbe7921f89db11efd1dba7a1f3a41685958110e2230f9c0ca1` |
| `linux/appimage/libfuse-LGPL2.txt` | libfuse `LGPL2.txt` at `fuse-3.15.0` | `dc626520dcd53a22f727af3ee42c770e56c97a64fe3adb063799d8ab032fe551` |
| `linux/appimage/squashfuse-LICENSE.txt` | vasi/squashfuse `LICENSE` at `0.5.2` | `9e909cc8a8ba27b1a649c964cf9eda37de911f3138c2b64e6d2f01703904ac13` |
| `linux/appimage/zstd-LICENSE.txt` | facebook/zstd `LICENSE` at `v1.5.6` (the BSD-3-Clause half of its dual license) | `7055266497633c9025b777c78eb7235af13922117480ed5c674677adc381c9d8` |
| `linux/appimage/zlib-LICENSE.txt` | madler/zlib `LICENSE` at `v1.3.2` | `e32ff4e00d9d94930537635291da39e7e612703334bf6fde8c7f1686fe8a45a2` |
| `linux/appimage/musl-COPYRIGHT.txt` | musl `COPYRIGHT` at `v1.2.5` | `f9bc4423732350eb0b3f7ed7e91d530298476f8fec0c6c427a1c04ade22655af` |
| `linux/appimage/AppImageKit-LICENSE` | AppImage/AppImageKit `LICENSE` at `1681fd84dbe09c7d9b22e13cdb16ea601aa0ec47` | `9cb22a08334c3a108ec4da96c1d6aea7a17a732fa9f5ca3413a9b4ce651397d8` |
| `linux/appimage/linuxdeploy-LICENSE.txt` | linuxdeploy/linuxdeploy `LICENSE.txt` | `1edb41ae7601854ae15d113c3700beebd0ea3ed05b85e3786a94a72bb9916091` |
| `linux/appimage/linuxdeploy-plugin-gtk-LICENSE.txt` | tauri-apps/linuxdeploy-plugin-gtk `LICENSE.txt` at `dda522b` | `0718f6cadf38d2662f397591b777884c5389e65b9919aae6aca2bea87a52eac4` |

The AppImage runtime texts match runtime `75849dc` (libfuse 3.15.0,
squashfuse 0.5.2, zstd 1.5.6, zlib 1.3.2), which the v1.6.2 AppImage embeds;
`scripts/appimage-notices.py` names the runtime and versions each build
actually embeds.
