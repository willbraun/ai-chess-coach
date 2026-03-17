<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';

	interface PvLine {
		rank: number;
		score: string;
		score_cp: number;
		moves: string;
	}

	interface Finding {
		text: string;
		priority: number;
	}

	interface PositionReport {
		material: string;
		tactics: Finding[];
		strategy: Finding[];
		summary: string;
	}

	interface Checkpoint {
		half_move: number;
		move_san: string;
		material: string;
		new_tactics: Finding[];
		removed_tactics: Finding[];
		new_strategy: Finding[];
		removed_strategy: Finding[];
		newly_attacked: Finding[];
	}

	interface LineComparison {
		engine_checkpoints: Checkpoint[];
		user_checkpoints: Checkpoint[];
	}

	interface AnalysisResult {
		best_move: string;
		lines: PvLine[];
		user_line: PvLine | null;
		position_report: PositionReport;
		comparison_text: string | null;
		line_comparison: LineComparison | null;
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
				args.userMove = userMove.trim().toLocaleLowerCase();
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

			<div class="report-section">
				<h3>Position Analysis</h3>
				<p class="material">{result.position_report.material}</p>

				{#if result.position_report.tactics.length > 0}
					<h4>Tactics</h4>
					<ul class="findings">
						{#each result.position_report.tactics as item (item.text)}
							<li class:critical={item.priority === 1}>{item.text}</li>
						{/each}
					</ul>
				{/if}

				{#if result.position_report.strategy.length > 0}
					<h4>Strategy</h4>
					<ul class="findings">
						{#each result.position_report.strategy as item (item.text)}
							<li class:critical={item.priority === 1}>{item.text}</li>
						{/each}
					</ul>
				{/if}

				{#if result.line_comparison}
					<div class="comparison-grid">
						<div class="comparison-column">
							<h4>User Line Checkpoints</h4>
							{#each result.line_comparison.user_checkpoints as cp (cp.half_move)}
								{@const hasChanges =
									cp.new_tactics.length > 0 ||
									cp.removed_tactics.length > 0 ||
									cp.new_strategy.length > 0 ||
									cp.removed_strategy.length > 0 ||
									cp.newly_attacked.length > 0}
								<div class="checkpoint">
									<span class="checkpoint-label">After {cp.move_san}</span>
									<p class="checkpoint-material">{cp.material}</p>
									{#if !hasChanges}
										<p class="no-findings">No changes</p>
									{/if}
									{#each cp.new_tactics as t (t.text)}
										<p class="finding tactic added">+ {t.text}</p>
									{/each}
									{#each cp.removed_tactics as t (t.text)}
										<p class="finding tactic removed">− {t.text}</p>
									{/each}
									{#each cp.new_strategy as s (s.text)}
										<p class="finding strategic added">+ {s.text}</p>
									{/each}
									{#each cp.removed_strategy as s (s.text)}
										<p class="finding strategic removed">− {s.text}</p>
									{/each}
									{#each cp.newly_attacked as a (a.text)}
										<p class="finding attacked">⚠ {a.text}</p>
									{/each}
								</div>
							{/each}
						</div>
						<div class="comparison-column">
							<h4>Engine Line Checkpoints</h4>
							{#each result.line_comparison.engine_checkpoints as cp (cp.half_move)}
								{@const hasChanges =
									cp.new_tactics.length > 0 ||
									cp.removed_tactics.length > 0 ||
									cp.new_strategy.length > 0 ||
									cp.removed_strategy.length > 0 ||
									cp.newly_attacked.length > 0}
								<div class="checkpoint">
									<span class="checkpoint-label">After {cp.move_san}</span>
									<p class="checkpoint-material">{cp.material}</p>
									{#if !hasChanges}
										<p class="no-findings">No changes</p>
									{/if}
									{#each cp.new_tactics as t (t.text)}
										<p class="finding tactic added">+ {t.text}</p>
									{/each}
									{#each cp.removed_tactics as t (t.text)}
										<p class="finding tactic removed">− {t.text}</p>
									{/each}
									{#each cp.new_strategy as s (s.text)}
										<p class="finding strategic added">+ {s.text}</p>
									{/each}
									{#each cp.removed_strategy as s (s.text)}
										<p class="finding strategic removed">− {s.text}</p>
									{/each}
									{#each cp.newly_attacked as a (a.text)}
										<p class="finding attacked">⚠ {a.text}</p>
									{/each}
								</div>
							{/each}
						</div>
					</div>

					<h4>Overall Comparison</h4>
					<p>{result.comparison_text}</p>
				{/if}
			</div>
		</div>
	{/if}
</main>

<style>
	main {
		max-width: 1000px;
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
		padding: 0.15rem 0.5rem;
		border-radius: 3px;
		box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
	}

	.positive {
		color: #1a1a1a;
		background: #ffffff;
	}

	.negative {
		color: #ffffff;
		background: #1a1a1a;
	}

	.moves {
		font-family: monospace;
		font-size: 0.85rem;
		color: #555;
		word-break: break-all;
	}

	.report-section {
		margin-top: 1.5rem;
		padding-top: 1rem;
		border-top: 1px solid #e5e7eb;
	}

	.report-section h3 {
		font-size: 1.1rem;
		margin-bottom: 0.5rem;
	}

	.report-section h4 {
		font-size: 0.95rem;
		margin-top: 0.75rem;
		margin-bottom: 0.25rem;
		color: #374151;
	}

	.material {
		font-weight: 500;
		margin-bottom: 0.5rem;
	}

	.findings {
		list-style: disc;
		padding-left: 1.25rem;
		font-size: 0.9rem;
		color: #444;
	}

	.findings li {
		margin-bottom: 0.25rem;
	}

	.checkpoint {
		background: #fafafa;
		border-left: 3px solid #d1d5db;
		padding: 0.5rem 0.75rem;
		margin-bottom: 0.5rem;
		border-radius: 0 4px 4px 0;
	}

	.checkpoint-label {
		font-size: 0.8rem;
		font-weight: 600;
		color: #6b7280;
		display: block;
		margin-bottom: 0.15rem;
	}

	.checkpoint-material {
		font-size: 0.8rem;
		color: #374151;
		margin: 0 0 0.25rem 0;
	}

	.no-findings {
		font-size: 0.85rem;
		color: #9ca3af;
		font-style: italic;
	}

	.finding {
		font-size: 0.85rem;
		margin: 0.15rem 0;
	}

	.finding.tactic {
		color: #b45309;
	}

	.finding.strategic {
		color: #4338ca;
	}

	.finding.attacked {
		color: #7c3aed;
	}

	.finding.removed {
		opacity: 0.55;
		text-decoration: line-through;
	}

	.comparison-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 1.5rem;
		margin-top: 0.5rem;
	}

	.comparison-column h4 {
		margin-top: 0;
		margin-bottom: 0.5rem;
	}
</style>
