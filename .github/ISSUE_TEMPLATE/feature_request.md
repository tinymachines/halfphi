---
name: Feature request
about: Something the library should be able to do
labels: enhancement
---

**What you are trying to find out about a chip**
This library is a set of questions you can ask a die. Describing the question is
usually more useful than describing the API you had in mind.

**Does it name a chip?**
If the feature needs to know what a clock edge is, which nodes are pins, or what
a register is, it probably belongs in a chip layer rather than here. That is not
a refusal — it is worth working out which side of the line it falls on early.
