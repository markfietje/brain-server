# Why I built the governance layer

*2026-09-11. A personal note behind the product. If you hire for support
operations, this is how I think.*

I have watched the same failure in every support operation I have
touched. Governance is side work. Nobody owns quality, access, or
cleanup, so quality rots, access sprawls, and cleanup never happens.
Then something breaks at 2 a.m. and everyone discovers the runbook was
in one person's head. That person was usually me, and I got tired of
being the runbook.

So I built the layer I kept wishing existed. Every write screened
before it lands. Every definition owned, with one meaning per thing.
Every recall carrying its lineage, so a wrong answer can be traced to
the exact row it came from. Access scoped per tool, so each integration
sees what it needs and nothing else. Deletion that produces a
certificate instead of a promise. An audit chain that answers "who
decided this was true" without a meeting.

None of this is exotic. It is warehouse discipline ported to agent
memory: screened writes, owned definitions, checked lineage, scoped
access, proof. The tools I name are different, Redshift and dbt and
Airflow do the heavy lifting in a warehouse, but the governance pattern
is the same one those tools need. A definition nobody owns drifts. A
write nobody checks poisons everything downstream. A deletion nobody can
prove is a liability with a date on it.

I work solo and hands on. I use Claude Code and Copilot daily to build
monitors and triage faster. Then I keep the rules in code, because code
does not forget, does not go on leave, and does not approve things at
4:55 p.m. on a Friday. When I say no to a write, the rule is written
down and the tooling makes yes easy. That is the whole philosophy. Make
the right action the cheap action, and one person can cover ground that
used to need a team.

This repository is the evidence. Every control named above runs here,
pinned by tests, with the ceilings stated where a buyer can price them.
If you are hiring for someone to own quality in an AI support
operation, start with the guarantees section of the README, then ask me
what broke to earn each one. I remember all of them.
