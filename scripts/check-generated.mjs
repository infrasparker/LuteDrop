// Regenerates the ts-rs bindings (via `cargo test`, which runs the #[ts(export)]
// exporters) and fails if `generated/` differs from what's committed. Run
// locally before pushing model changes. CI does not exist yet; when it does,
// wire this in as a step (documented in package.json).
import { execSync } from 'node:child_process';

execSync('cargo test --manifest-path src-tauri/Cargo.toml', { stdio: 'inherit' });
try {
  execSync('git diff --exit-code -- generated', { stdio: 'inherit' });
  console.log('generated/ is in sync.');
} catch {
  console.error('\nERROR: generated/ is out of date. Commit the regenerated bindings.');
  process.exit(1);
}
