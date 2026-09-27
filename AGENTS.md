# Instructions for AI agents

These rules apply to an AI agent working in any tectonic-os repository. They
restate the project's
[AI use policy](https://github.com/tectonic-os/.github/blob/main/AI_POLICY.md)
for the agent.

- The human who submits the change is its author. Never list an AI as a
  co-author, and never add `Signed-off-by`. Only the human adds it.
- A commit with substantial help from you carries the trailer
  `Assisted-by: AI`, and a commit you effectively wrote whole carries
  `Generated-by: AI`. Neither names a model or a tool.
- One branch and one pull request hold one feature. The pull request title is a
  typed subject, `<type>(<scope>): <description>`, of at most 64 characters.
  The [contributing guide](https://github.com/tectonic-os/.github/blob/main/CONTRIBUTING.md)
  lists the types.
- Run the checks the repository's CI runs before you propose a change, and
  report their result as it is.
- A generated change of more than about 500 lines starts as a design issue.
- Issues, reviews and discussions are the human's, in the human's own words.
  Draft them only when the human asks, and say that the draft is one.
