# Keep Agent Goal Intake outside the scanner

Accepted. Code Intel Pipeline supports Agent Goal Intake as an upstream task-contract concept, using goal-authoring tools such as `qiaomu-goal-meta-skill` and loop-pattern catalogs such as `awesome-agent-loops` to shape bounded `/goal`, `/loop`, or `/schedule` work before execution. These concepts must not be wired into the scanner runtime: the scanner owns fresh repository evidence and artifact runs, while goal and loop intake own task framing, verification boundaries, iteration policy, and pause conditions.
