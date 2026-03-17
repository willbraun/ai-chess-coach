<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';

	interface PvLine {
		rank: number;
		score: string;
		score_cp: number;
		moves: string;
	}

	interface AnalysisResult {
		best_move: string;
		lines: PvLine[];
		user_line: PvLine | null;
	}

	let fen = $state('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1');
	let userMove = $state('');
	let result: AnalysisResult | null = $state(null);
	let error: string | null = $state(null);
	let loading = $state(false);

	async function analyze() {
		loading = true;
		error = null;
		result = null;
		try {
			const args: { fen: string; userMove?: string } = { fen };
			if (userMove.trim()) {
				args.userMove = userMove.trim();
			}
			result = await invoke<AnalysisResult>('analyze_position', args);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}
</script>

<main>
	<h1>AI Chess Coach</h1>

	<div class="input-group">
		<label for="fen">FEN</label>
		<input id="fen" type="text" bind:value={fen} placeholder="Enter FEN string" />
		<label for="user-move">Your Move (UCI format, e.g. e2e4)</label>
		<input
			id="user-move"
			type="text"
			bind:value={userMove}
			placeholder="Optional — e.g. e2e4, g1f3"
		/>
		<button onclick={analyze} disabled={loading}>
			{loading ? 'Analyzing...' : 'Analyze'}
		</button>
	</div>

	{#if error}
		<p class="error">{error}</p>
	{/if}

	{#if result}
		<div class="results">
			<h2>Best Move: {result.best_move}</h2>

			{#if result.user_line}
				<div class="pv-line user-line">
					<div class="pv-header">
						<span class="rank">Your Move</span>
						<span
							class="score"
							class:positive={result.user_line.score_cp > 0}
							class:negative={result.user_line.score_cp < 0}
						>
							{result.user_line.score}
						</span>
					</div>
					<p class="moves">{result.user_line.moves}</p>
				</div>
			{/if}

			{#each result.lines as line (line.rank)}
				<div class="pv-line">
					<div class="pv-header">
						<span class="rank">Line {line.rank}</span>
						<span
							class="score"
							class:positive={line.score_cp > 0}
							class:negative={line.score_cp < 0}
						>
							{line.score}
						</span>
					</div>
					<p class="moves">{line.moves}</p>
				</div>
			{/each}
		</div>
	{/if}
</main>

<style>
	main {
		max-width: 700px;
		margin: 0 auto;
		padding: 2rem;
		font-family: system-ui, sans-serif;
	}

	h1 {
		margin-bottom: 1.5rem;
	}

	.input-group {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		margin-bottom: 1.5rem;
	}

	.input-group input {
		padding: 0.5rem;
		font-size: 0.9rem;
		font-family: monospace;
		border: 1px solid #ccc;
		border-radius: 4px;
	}

	.input-group button {
		align-self: flex-start;
		padding: 0.5rem 1.5rem;
		font-size: 1rem;
		cursor: pointer;
		background: #2563eb;
		color: white;
		border: none;
		border-radius: 4px;
	}

	.input-group button:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}

	.error {
		color: #dc2626;
		font-weight: 500;
	}

	.results h2 {
		font-size: 1.2rem;
		margin-bottom: 1rem;
	}

	.pv-line {
		background: #f8f8f8;
		border-radius: 6px;
		padding: 0.75rem 1rem;
		margin-bottom: 0.75rem;
	}

	.user-line {
		background: #eff6ff;
		border: 1px solid #bfdbfe;
	}

	.pv-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 0.25rem;
	}

	.rank {
		font-weight: 600;
	}

	.score {
		font-weight: 700;
		font-size: 1.1rem;
		font-family: monospace;
	}

	.positive {
		color: #16a34a;
	}

	.negative {
		color: #dc2626;
	}

	.moves {
		font-family: monospace;
		font-size: 0.85rem;
		color: #555;
		word-break: break-all;
	}
</style>
