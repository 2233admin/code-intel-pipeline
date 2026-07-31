export default {
  integrationBranch: "codex/code-intel-atomic-model",
  productionBranch: "main",
  protectedBranches: [],
  regenerableFiles: [],
  disposableUntracked: [],
  symlinks: ["node_modules"],
  buildOutputDirs: ["target", "artifacts", "dist"],
  checkCommand: "pwsh -NoProfile -File ./legacy/Invoke-CodeIntelAcceptance.ps1 -Stage land",
  checksRequired: true,
};
