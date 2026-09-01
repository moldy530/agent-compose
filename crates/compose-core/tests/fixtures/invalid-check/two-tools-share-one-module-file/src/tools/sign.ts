// The fixture's subject is the shared file, so the file exists: a missing
// implementation is a different rule with a fixture of its own.
export default async function sign(): Promise<{ signature: string }> {
  return { signature: "" };
}
