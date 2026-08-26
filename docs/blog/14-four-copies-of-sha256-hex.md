# Four copies of sha256_hex: what happened when we let a machine audit our own repo

*2026. We pointed an agent at our own documentation and source tree, asked one question, "is any of this still true?", and got back a list long enough to change how the whole project treats its helpers, human and otherwise.*

Every repo has two versions of itself. The one in the docs, confident and tidy, and the one on disk, which has been quietly drifting since the day after the docs were written. Most weeks nobody notices. Then somebody follows the trust walkthrough, the document whose entire job is proving the security claims are real, and it sets an environment variable called `BRAIN_PORT` that has never existed in the codebase. The server ignores it, binds to the default port anyway, and every curl in the walkthrough misses from step zero.

We know because we ran that experiment on ourselves.

This post is what came out of it: a full reverse audit of every living page against the actual source, a handful of fixes that mattered, and then a harder question. If our docs could lie to us for seventeen releases, what else was the repo quietly believing? The answer involved four independent copies of a hashing helper, a function pasted twice inside a single file, a roadmap that thought the year stopped in August, and a YouTube video from IBM that turned out to describe our week better than we could.

## The audit, briefly

We extracted the facts from the source first. Every route the router registers, every environment variable the config reads, every subcommand the CLI dispatches. Then we scraped the documentation for claims and diffed the two lists. The method sounds boring because it is, and that is the point. Machines are wonderful at boring.

The findings fell into three buckets. Broken instructions: the trust script above, plus an approve example in the quickstart that would get a 400 error since release 1.27.12 added a required digest parameter. Wrong facts: the roadmap announcing release 1.28.17 as the newest thing alive when Cargo.toml said otherwise, a security page describing the audit chain without mentioning it grew keyed HMAC links months ago, and a features page claiming exactly one connector binary exists when a second one shipped with three CRM backends behind it. And quiet drift, the kind that never breaks anything but slowly rots: version stamps frozen at older releases, a benchmark header still shouting that all results were pending, above tables of results that had been sitting there for weeks.

None of this was malice or even sloppiness, really. It was the ordinary entropy of a fast-moving project where the code gets a test gate and the prose does not. Fixing the text took an afternoon. Deciding it would not happen again took longer, and that decision is the rest of this post.

## A video said it out loud

Around the time we finished, [IBM Technology published a piece called "How AI Coding Agents Understand Your Codebase & Developer Tools."](https://www.youtube.com/watch?v=zAe-sau06io) Watch it if you work with coding agents, because it names the failure mode precisely: these tools are very good at producing code that runs, and very bad, by default, at producing code that belongs. The presenter's example is a service layer. Every database call is supposed to go through it, because that is where permissions and logging live. Ask an agent for a new endpoint and it may happily write the query straight into the handler. It works. Tests pass. And the system just got worse, because now there are two ways to touch the data and one of them skips the rules.

The video proposes five habits for tools that respect a codebase. Repo awareness, meaning finding the right context rather than dumping everything into the prompt. Architectural context, meaning the unwritten rules about where logic goes. Planning before patching, so the first output is reasoning instead of a diff. Verification that asks whether a change fits, not merely whether it compiles. And boundaries, which the presenter summarizes as manners: the tool should knock first.

Here is what struck us. Those five habits are not agent features you wait for. They are repo properties you can build. An agent can only respect rules it cannot break, and a repo can make its rules unbreakable.

## The research agrees, mostly uncomfortably

None of this is vibes. There is a decade of empirical software engineering behind each pillar, and lately some very uncomfortable numbers about AI specifically.

Start with duplication. GitClear analyzes enormous corpora of changed lines, 153 million for [the 2024 report](https://www.gitclear.com/coding_on_copilot_data_shows_ais_downward_pressure_on_code_quality/) and 211 million for [the 2025 follow-up](https://www.gitclear.com/ai_assistant_code_quality_2025_research), drawn partly from Google, Microsoft, and Meta repositories. Their headline findings: code churn, lines reverted or rewritten within two weeks, roughly doubled against the pre-AI baseline, and duplicated blocks grew around four times faster in 2024 than in 2021. Copy-pasted lines overtook moved lines for the first time in their dataset, while refactoring collapsed from about a quarter of changed code in 2021 to under ten percent in 2024. Their phrasing for AI-generated code sticks with me: it resembles an itinerant contributor, prone to violate the DRY-ness of the repos visited. Assistants suggest additions, never consolidations, so the mess compounds.

Does catching that stuff early matter? The code review literature says yes, with a twist most teams ignore. [Bacchelli and Bird](https://doi.org/10.1109/ICSE.2013.6606617) studied hundreds of review comments across Microsoft teams and found that defects, the stated reason reviews exist, made up only about fourteen percent of the comments. The bulk was smaller stuff, and the hardest part of reviewing turned out to be understanding the change at all. Their recommendation reads like a to-do list for our week: automate the mechanical checks so human attention goes to design. [McIntosh and colleagues](https://rebels.cs.uwaterloo.ca/papers/emse2016_mcintosh.pdf) went further across Qt, VTK, and ITK, showing that review coverage, participation, and expertise track post-release defects in large systems. [Google's own study of nine million reviewed changes](https://research.google/pubs/modern-code-review-a-case-study-at-google/) describes the machine they built to keep changes small and feedback fast. In other words, the industry already knows reviewers are wasted on lint and missed context. We just kept paying them anyway.

Then there is the speed question, and here the recent research turns genuinely heretical. [A randomized trial by METR](https://metr.org/blog/2025-07-10-early-2025-ai-experienced-os-dev-study/) followed sixteen experienced open-source developers through 246 real issues on projects they knew intimately, some with five years of history in the repo. Randomly assigned issues could use frontier AI tools or not. Result: the AI group took nineteen percent longer. Better still, those developers forecast a twenty-four percent speedup beforehand, and even after being slowed down, they estimated they had been sped up by twenty percent. Perception and reality parted ways completely. [DORA's 2024 survey](https://dora.dev/research/2024/dora-report/) of nearly forty thousand professionals points the same direction from the other side: as AI adoption rose, delivery stability dropped an estimated 7.2 percent per 25 percent increase in adoption, and the researchers' leading hypothesis is that generated code is quietly exploding batch sizes, which decades of DORA data tie directly to instability.

Read those together and the pattern is hard to miss. On mature codebases with high standards, the bottleneck is not typing. It is knowing which of the four existing copies of the utility to call, what the architecture forbids, and what the docs promised last quarter. Exactly the things an eager assistant does not check unless something forces it to.

## So we forced it

Everything below is now enforced by tests that fail CI, not by policy documents that hope.

Docs tell the truth or the build stays red. A tiny module holds pins that read specific pages and assert specific facts, including one that checks the metrics dictionary documents a config default the code actually uses. When a standards body revises a document we cite, the pin fails until a human re-reads and re-maps deliberately. Boring, mechanical, effective.

Structure has a ledger. The monolith problem in our main binary, nineteen thousand five hundred lines at last count, two thirds of it tests, is scheduled for extraction across named releases, but the inventory guard landed first. Line counts, route counts, and test counts are pinned constants that may only move in one direction. Growth needs a reviewed edit. Shrinkage earns itself.

Duplication got its own gate, and the gate earned its keep on day one. It walks the source tree, collects every top-level function name, and fails when the same name is defined in more than one file without a reasoned exemption. First run: `sha256_hex` defined four separate times, in backup, knowledge base, mesh, and parcels. `set_mode_0600` twice, in the same file, thirty lines apart. A domain validator duplicated next to the module whose doc comment declares itself the single source of truth. Fifty-eight collisions in total. Each is now either scheduled for extraction or documented with a reason that must survive its own staleness test, and the exempted count can only shrink in reviewable diffs.

Unused dependencies got the same treatment, via a dependency analyzer wired into CI. Its debut found a networking library declared directly and imported nowhere, plus two more dead weights in satellite crates. Gone the same day.

And the workflow around every change now matches the video's sequence, read, plan, patch, verify, review. Our execution prompts open with a re-verification list: here are the exact files and line numbers this plan assumes, confirm them before touching anything, and if reality has drifted, stop and update the plan in the same commit. Boundaries are explicit, named sections listing what may not be touched. Verification means the full matrix, format, lints at deny level across five build surfaces, tests everywhere including the engine crates, a changed-line diagnostics gate, byte-diffs on the wire contract, and a smoke run against a copy of the production database ending in a verified audit chain.

## What the gates cannot do

Honesty requires the ceiling paragraph, because a vendor blog that only sells certainty is selling something else.

Name-based duplicate detection catches clones, not cousins. Two helpers doing subtly different things under one name will pass until someone unifies them and discovers the difference the hard way. The allowlist is a debt registry, not a pardon; every entry marked as pending unification is a public admission, and the count only moves in the direction of fewer.

Gates catch shape, not intent. A change can satisfy every pin and still be the wrong change, aimed at a problem the architecture was not asking to solve. That judgment stays human, and the research explains why: understanding remains the irreducible cost, whether the reader is paid by the hour or measured in tokens. The METR result cuts both ways and we take it seriously. Agents slowed down experts precisely where context was deepest, which is another way of saying familiarity is the asset, and no prompt yet substitutes for it. Our bet is narrower than "AI writes our code." It is that a repo which encodes its own rules can accept help from anything, silicon or otherwise, without slowly forgetting what it meant.

The docs lie to you for exactly as long as nothing checks them. The codebase duplicates itself for exactly as long as nothing counts. Neither fact requires a clever fix. Both require a stubborn one.

Knock first.

## Sources

- IBM Technology, ["How AI Coding Agents Understand Your Codebase & Developer Tools"](https://www.youtube.com/watch?v=zAe-sau06io) (2026). The five habits: repo awareness, architectural context, plan before patch, verify fit, boundaries.
- Harding and Kloster, GitClear, ["Coding on Copilot: 2023 Data Shows Downward Pressure on Code Quality"](https://www.gitclear.com/coding_on_copilot_data_shows_ais_downward_pressure_on_code_quality/) (2024), [open-access PDF mirror](https://gwern.net/doc/ai/nn/transformer/gpt/codex/2024-harding.pdf). 153 million changed lines; churn projected to double; copy/paste up 11.3 percent year over year while moved code fell 17.3 percent.
- GitClear, ["AI Copilot Code Quality: 2025 Look Back at 12 Months of Data"](https://www.gitclear.com/ai_assistant_code_quality_2025_research) (2025). 211 million changed lines; 4x growth in duplicate blocks; copy/paste exceeds moved code for the first time; refactoring share under ten percent of changed lines.
- Bacchelli and Bird, ["Expectations, Outcomes, and Challenges of Modern Code Review"](https://doi.org/10.1109/ICSE.2013.6606617), ICSE 2013. Defect comments are roughly one in seven; understanding is the hard part; automate the mechanical checks.
- McIntosh, Kamei, Adams, and Hassan, ["An Empirical Study of the Impact of Modern Code Review Practices on Software Quality"](https://rebels.cs.uwaterloo.ca/papers/emse2016_mcintosh.pdf), Empirical Software Engineering 2016. Review coverage, participation, and expertise correlate with post-release defects across Qt, VTK, and ITK.
- Sadowski, Söderberg, Church, Sipko, and Bacchelli, ["Modern Code Review: A Case Study at Google"](https://research.google/pubs/modern-code-review-a-case-study-at-google/), ICSE SEIP 2018. Nine million reviewed changes; small changes, fast feedback, automation under the human layer.
- Becker, Rush, Barnes, and Rein, METR, ["Measuring the Impact of Early-2025 AI on Experienced Open-Source Developer Productivity"](https://metr.org/blog/2025-07-10-early-2025-ai-experienced-os-dev-study/) (2025), [preprint](https://arxiv.org/abs/2507.09089). Randomized trial: nineteen percent slower with AI, while developers believed twenty percent faster.
- Google Cloud, [DORA Accelerate State of DevOps 2024](https://dora.dev/research/2024/dora-report/). Nearly forty thousand respondents; estimated 7.2 percent delivery-stability reduction per 25 percent increase in AI adoption; batch-size hypothesis.
