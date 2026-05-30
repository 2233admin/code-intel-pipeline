#include "tree_sitter/parser.h"

const TSLanguage *tree_sitter_v(void);

#ifdef _WIN32
__declspec(dllexport)
#endif
const TSLanguage *tree_sitter_vlang(void) {
  return tree_sitter_v();
}
