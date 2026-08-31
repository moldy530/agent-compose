// The first spelling, on disk. The second one deliberately is not: on the hosts
// this rule is about it could not be a second file, and on the host this corpus
// runs on its presence would say nothing — the check reads the composition
// rather than the filesystem.
export default async function sign(): Promise<{ signature: string }> {
  return { signature: "" };
}
