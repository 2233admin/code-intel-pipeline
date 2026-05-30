# Third-Party Notes

The Windows grammar DLL is built from `nedpals/tree-sitter-v`, which is published under the MIT license.

- Source: https://github.com/nedpals/tree-sitter-v
- npm package checked: `tree-sitter-v@1.0.7`
- Grammar ABI: 13
- Windows DLL SHA256: `921dec08ca60455fca2794148bee852f91e2d2aa8853a85ca62a42bccdcf216f`
- License copy: `LICENSE.tree-sitter-v`

Local wrapper code in `src/sentrux_vlang_alias.c` exports `tree_sitter_vlang()` for Sentrux compatibility and delegates to the upstream `tree_sitter_v()` parser.
