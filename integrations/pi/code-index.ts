/**
 * code-index-cli tool for pi (https://github.com/janih/code-index-cli).
 *
 * Exposes semantic code search over projects indexed by code-index-cli.
 * Pairs with ~/.config/code-index/projects.json and
 * examples/multi-project-watch/ in this repo, which is what keeps those
 * projects' indexes fresh via `code-index watch`.
 *
 * - code_index_search   — semantic search scoped to one project
 * - code_index_projects — list the projects currently configured
 *
 * Projects are named by the basename of their path in
 * ~/.config/code-index/projects.json, so an agent can ask about "spm"
 * without needing to know its absolute path. A project not in that list
 * can still be queried via the "workspace" parameter (absolute path).
 *
 * Install: see integrations/pi/README.md in this repo.
 */

import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, formatSize, truncateHead } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const PROJECTS_FILE = join(homedir(), ".config", "code-index", "projects.json");
const DEFAULT_LIMIT = 8;

function resolveBinary(): string {
	// pi may run with a narrower PATH than an interactive shell (the same
	// issue launchd hits — see examples/multi-project-watch/README.md), so
	// check the common install locations before falling back to PATH.
	const candidates = [
		join(homedir(), ".cargo", "bin", "code-index"),
		"/opt/homebrew/bin/code-index",
		"/usr/local/bin/code-index",
	];
	for (const c of candidates) {
		if (existsSync(c)) return c;
	}
	return "code-index";
}

const BINARY = resolveBinary();

function loadProjects(): Record<string, string> {
	try {
		const raw = readFileSync(PROJECTS_FILE, "utf8");
		const paths = JSON.parse(raw) as string[];
		const map: Record<string, string> = {};
		for (const p of paths) map[basename(p)] = p;
		return map;
	} catch {
		return {};
	}
}

interface Payload {
	filePath: string;
	codeChunk: string;
	startLine: number;
	endLine: number;
	[key: string]: unknown;
}

interface SearchResult {
	id: unknown;
	score: number;
	payload: Payload | null;
}

function stripAt(value: string): string {
	// Some models prepend a stray "@" to path-like arguments.
	return value.startsWith("@") ? value.slice(1) : value;
}

function runCodeIndex(args: string[], signal?: AbortSignal): Promise<string> {
	return new Promise((resolve, reject) => {
		execFile(
			BINARY,
			args,
			{ maxBuffer: 20 * 1024 * 1024, timeout: 120_000, signal },
			(err, stdout, stderr) => {
				if (err) {
					if ((err as NodeJS.ErrnoException).code === "ENOENT") {
						reject(new Error(`code-index binary not found (looked for it at "${BINARY}"). Is it installed and on PATH?`));
						return;
					}
					reject(new Error(`code-index ${args[0]} failed: ${(stderr || err.message).trim()}`));
					return;
				}
				resolve(stdout);
			},
		);
	});
}

export default function (pi: ExtensionAPI) {
	const projectsAtLoad = loadProjects();
	const knownNames = Object.keys(projectsAtLoad);

	pi.registerTool({
		name: "code_index_projects",
		label: "Code Index Projects",
		description: `List the projects currently available to code_index_search (reads ${PROJECTS_FILE} fresh, so it reflects projects added after this extension loaded).`,
		promptSnippet: "List which projects are available to code_index_search",
		parameters: Type.Object({}),
		async execute() {
			const projects = loadProjects();
			const names = Object.keys(projects);
			const text = names.length
				? names.map((n) => `${n} — ${projects[n]}`).join("\n")
				: `No projects configured in ${PROJECTS_FILE}.`;
			return {
				content: [{ type: "text", text }],
				details: { projects },
			};
		},
	});

	pi.registerTool({
		name: "code_index_search",
		label: "Code Index Search",
		description:
			"Semantic (embedding-based) code search over one indexed project via code-index-cli. " +
			'Pass "project" as a configured project name (see code_index_projects for the current ' +
			'list), or "workspace" as an absolute path for a project not yet configured. ' +
			(knownNames.length
				? `Known projects as of extension load: ${knownNames.join(", ")}.`
				: `No projects were configured in ${PROJECTS_FILE} when this extension loaded.`),
		promptSnippet: "Semantic search over one indexed project's codebase",
		promptGuidelines: [
			'Use code_index_search for conceptual or "how does X work" questions about a specific ' +
				"project's codebase instead of grep, when that project has been indexed.",
			"Use code_index_projects with code_index_search to see which project names are currently valid.",
			'If code_index_search errors with "Unknown project", call code_index_projects and retry with one of the names it returns instead of guessing or giving up.',
			"code_index_search reads whatever a separate `code-index watch` process last indexed, not the file on disk right now — a stale-looking result is a cue to double-check with read or grep, not proof the code doesn't exist.",
		],
		parameters: Type.Object({
			project: Type.Optional(
				Type.String({ description: "Configured project name (basename of its path in projects.json)" }),
			),
			workspace: Type.Optional(
				Type.String({ description: "Absolute path to search directly, bypassing the configured project list" }),
			),
			query: Type.String({ description: "Natural-language search query" }),
			directory: Type.Optional(
				Type.String({ description: "Restrict results to this sub-directory of the project" }),
			),
			limit: Type.Optional(
				Type.Integer({ description: `Max results (default ${DEFAULT_LIMIT})`, minimum: 1, maximum: 50 }),
			),
		}),

		async execute(_toolCallId, params, signal) {
			const projects = loadProjects(); // fresh read: picks up edits made after registration

			let workspace = params.workspace ? stripAt(params.workspace) : undefined;
			if (!workspace) {
				if (!params.project) {
					throw new Error('Pass either "project" (a configured project name) or "workspace" (an absolute path).');
				}
				const name = stripAt(params.project);
				workspace = projects[name];
				if (!workspace) {
					const available = Object.keys(projects);
					throw new Error(
						`Unknown project "${name}". Configured projects: ${
							available.length ? available.join(", ") : `(none — see ${PROJECTS_FILE})`
						}`,
					);
				}
			}

			const limit = params.limit ?? DEFAULT_LIMIT;
			const args = ["search", "-w", workspace, "-q", params.query, "--format", "json", "-n", String(limit)];
			if (params.directory) args.push("--directory", stripAt(params.directory));

			const stdout = await runCodeIndex(args, signal);

			let results: SearchResult[];
			try {
				results = JSON.parse(stdout);
			} catch {
				throw new Error(`code-index returned non-JSON output: ${stdout.slice(0, 500)}`);
			}

			if (results.length === 0) {
				return {
					content: [{ type: "text", text: `No results for "${params.query}" in ${workspace}.` }],
					details: { workspace, query: params.query, resultCount: 0 },
				};
			}

			const blocks = results.map((r, i) => {
				if (!r.payload) return `${i + 1}. [${(r.score * 100).toFixed(1)}%] (no payload)`;
				const p = r.payload;
				return `${i + 1}. [${(r.score * 100).toFixed(1)}%] ${p.filePath}:${p.startLine}-${p.endLine}\n${p.codeChunk}`;
			});
			const header = `Search results for "${params.query}" in ${workspace} (${results.length}):\n\n`;

			const truncation = truncateHead(header + blocks.join("\n\n"), {
				maxLines: DEFAULT_MAX_LINES,
				maxBytes: DEFAULT_MAX_BYTES,
			});
			let text = truncation.content;
			if (truncation.truncated) {
				text +=
					`\n\n[Output truncated: ${truncation.outputLines} of ${truncation.totalLines} lines ` +
					`(${formatSize(truncation.outputBytes)} of ${formatSize(truncation.totalBytes)}). ` +
					`Narrow the query or pass a lower "limit".]`;
			}

			return {
				content: [{ type: "text", text }],
				details: { workspace, query: params.query, resultCount: results.length, results },
			};
		},
	});
}
