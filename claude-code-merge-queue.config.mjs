export default {
  integrationBranch: "codex/code-intel-atomic-model",
  productionBranch: "main",
  protectedBranches: [],
  regenerableFiles: [],
  disposableUntracked: [],
  symlinks: ["node_modules"],
  buildOutputDirs: ["target", "artifacts", "dist"],
  checkCommand: "cargo +stable test -p code-intel --locked",
  checksRequired: true,
};
