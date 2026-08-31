// The fixture's subject is the dependency conflict, so the files exist: a
// missing implementation is a different rule with a fixture of its own.
export default async function verify(): Promise<{ ok: boolean }> {
  return { ok: true };
}
