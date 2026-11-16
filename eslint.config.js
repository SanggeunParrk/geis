// Lint for the frontend. The compiler already runs strict; this adds the rules that need
// types to see what the compiler does not complain about - above all a promise nobody is
// waiting on, which in this app is an action that fails without a word.
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src-tauri", "vendor", "output", "*.config.*"] },
  ...tseslint.configs.recommendedTypeChecked,
  {
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: {
      // The codebase's own conventions, which the compiler enforces already.
      "@typescript-eslint/no-unused-vars": "off",
      // `void promise` is how a deliberate fire-and-forget is written here.
      "@typescript-eslint/no-floating-promises": ["error", { ignoreVoid: true }],
      // The agent's host interface is async throughout, and many of its methods answer at
      // once; an `async` with nothing to await is that interface being implemented.
      "@typescript-eslint/require-await": "off",
      "@typescript-eslint/no-misused-promises": ["error", { checksVoidReturn: false }],
    },
  },
);
