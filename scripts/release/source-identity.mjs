import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseVersion, readJson, validateSourceCommit, validateTag } from "./common.mjs";

export function sourceIdentity({
  repository = process.cwd(), product, tag, publish = false, eventName, refType, refName,
}, runGit = args => execFileSync("git", args, {
  cwd: repository, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
}).trim()) {
  if (!["app", "sync"].includes(product)) throw new Error("invalid-release-product");
  if (typeof publish !== "boolean") throw new Error("invalid-publication-mode");
  if (publish && !tag) throw new Error("publication-tag-required");
  const version = readJson(join(repository, product === "app" ? "version.json" : "server/version.json")).version;
  parseVersion(version);
  tag ||= `${product}-v${version}`;
  validateTag(product, tag, version);

  const sourceCommit = validateSourceCommit(runGit(["rev-parse", "HEAD"]));
  runGit(["merge-base", "--is-ancestor", sourceCommit, "refs/remotes/origin/main"]);
  if (eventName === "push" && (refType !== "tag" || tag !== refName)) {
    throw new Error("pushed-release-tag-mismatch");
  }
  if (publish || eventName === "push") {
    const tagged = runGit(["rev-parse", "--verify", `refs/tags/${tag}^{commit}`]);
    if (tagged !== sourceCommit) throw new Error("release-tag-commit-mismatch");
  }
  return { tag, sourceCommit };
}

function main() {
  if (!["true", "false"].includes(process.env.PUBLISH)) throw new Error("invalid-publication-mode");
  const identity = sourceIdentity({
    product: process.env.PRODUCT,
    tag: process.env.TAG,
    publish: process.env.PUBLISH === "true",
    eventName: process.env.GITHUB_EVENT_NAME,
    refType: process.env.GITHUB_REF_TYPE,
    refName: process.env.GITHUB_REF_NAME,
  });
  appendFileSync(process.env.GITHUB_OUTPUT, `tag=${identity.tag}\nsource_commit=${identity.sourceCommit}\n`);
  console.log(JSON.stringify(identity));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
