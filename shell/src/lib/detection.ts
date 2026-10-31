/**
 * W-T2 — the detection report's wire shapes (the Rust core owns every law;
 * `detect.rs` is the ONE implementation, and these interfaces are what its
 * DTO looks like over the IPC seam). The proposal cards render THIS — no
 * model, no generation: the cards are if-branches over a local DTO whose
 * every claim carries the path it was read from.
 */

export interface ExistingInstallDto {
	binaryPath: string;
	version: string | null;
	running: boolean;
	evidence: string;
}

export interface OpenClawDto {
	pluginEnabled: boolean;
	evidence: string;
}

/** D3's listing: name, size, mtime — the shape has NO contents field, so
 * the installer could not carry the user's memory out even by accident. */
export interface WorkspaceDbDto {
	name: string;
	sizeBytes: number;
	modifiedEpoch: number;
}

export interface WorkspaceDto {
	databases: WorkspaceDbDto[];
	totalBytes: number;
	evidence: string;
}

export interface KernelOnPortDto {
	running: boolean;
	version: string | null;
	holder: string | null;
	evidence: string;
}

export interface UpdateDto {
	binary: string;
	installedVersion: string | null;
	offeredVersion: string | null;
	evidence: string;
}

export interface DetectDto {
	existing: ExistingInstallDto | null;
	openclaw: OpenClawDto | null;
	workspace: WorkspaceDto | null;
	kernelOnPort: KernelOnPortDto;
	updates: UpdateDto[];
}

/** The C-prefill: the kernel the probe found is the kernel posture C
 * attaches to (loopback only, this release). */
export const LOOPBACK_KERNEL_URL = 'http://127.0.0.1:8765';

export function formatBytes(total: number): string {
	if (total < 1024) return `${total} B`;
	if (total < 1024 * 1024) return `${Math.round(total / 1024)} KB`;
	return `${(total / (1024 * 1024)).toFixed(1)} MB`;
}
