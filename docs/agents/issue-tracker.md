# Issue Tracker: Linear

Issues and specifications for this repository live in the Greg Headley team's
[ObsCam project](https://linear.app/headcase/project/obscam-0488d6d1e204) in
Linear. Use the connected Linear integration for all tracker operations.

## Conventions

- Team: `Greg Headley` (`GRE`)
- Project: `ObsCam`
- Create, read, update, comment on, and close issues through Linear.
- Use native parent/sub-issue and blocking relationships.
- Use issue labels for triage and skill workflow state.

## Publish To The Issue Tracker

Create a Linear issue in the `Greg Headley` team and `ObsCam` project.

## Fetch A Ticket

Fetch the Linear issue by its `GRE-<number>` identifier, including relations and
comments when they are relevant.

## Wayfinding Operations

- **Map**: an ObsCam issue labelled `wayfinder:map`.
- **Child ticket**: an ObsCam sub-issue of the map, labelled
  `wayfinder:<type>`.
- **Blocking**: Linear's native `blockedBy` relationship.
- **Frontier**: open, unassigned, unblocked children of the map, in issue order.
- **Claim**: assign the ticket to the current Linear user before work.
- **Resolve**: add the answer as a comment, close the ticket, then append a
  linked gist to the map's Decisions-so-far.
