---
name: Feature request
about: Suggest an enhancement for brain-server or the brain-client
title: "[feature] "
labels: enhancement
assignees: ""
---

## Summary

A clear, concise description of the feature you're proposing.

## Motivation / use case

Why is this needed? What problem does it solve, and for whom?

## Proposed behavior

What should happen once implemented. If there's an API surface involved, sketch
the endpoint/params/response you have in mind.

## Fit with the roadmap

- Have you checked `ROADMAP.md` / the `IMPLEMENTATION_PLAN_*.md` files? Is this
  already planned in a future release (e.g. v1.17.x, v2.0)?
- If it's already planned, does this request refine the plan?

## Considerations

- **Low-power constraint:** brain-server runs on ARM/low-watt hardware and
  avoids LLM calls in the hot path. Will this feature add measurable CPU/RSS?
- **Dependency cost:** does it require a new crate? (New deps need strong
  justification.)
- **Security/AuthZ:** does it expose a new surface that needs gating?
