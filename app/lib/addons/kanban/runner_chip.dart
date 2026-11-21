// A card's runner chip, "Laptop · waiting", with the assignment's reason
// underneath when there is one, then the agent's progress line. The chip and
// each line stay on one line and end in an ellipsis; the full text is in a
// tooltip.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'runner_backend.dart';

/// The chip's words: the runner's name, the project's name when the task has
/// one, then the state, each after a dot.
String runnerChipText(RunnerAssignment assignment) {
  final project = assignment.projectName;
  return [
    assignment.runnerName,
    if (project != null && project.isNotEmpty) project,
    assignment.note == null
        ? assignment.state.label
        : '${assignment.state.label} (${assignment.note})',
  ].join(' · ');
}

/// The progress line under the chip, or null when there is none to show. It
/// shows while the run is going and after a failed or cancelled run, where it
/// keeps the last text. A done run shows none.
String? runnerProgressText(RunnerAssignment assignment) {
  final text = assignment.progress?.trim();
  if (text == null || text.isEmpty) return null;
  return switch (assignment.state) {
    AssignState.running ||
    AssignState.stalled ||
    AssignState.failed ||
    AssignState.cancelled => text,
    _ => null,
  };
}

class RunnerChip extends StatelessWidget {
  const RunnerChip({super.key, required this.assignment});

  final RunnerAssignment assignment;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = assignment;
    final Color background = switch (a.state) {
      AssignState.queued ||
      AssignState.cancelling ||
      AssignState.cancelled => c.bgSecondary,
      AssignState.waiting ||
      AssignState.review ||
      AssignState.needsInput ||
      AssignState.stalled => c.attnBg,
      AssignState.running => c.infoBg,
      AssignState.done => c.okBg,
      AssignState.failed => c.dangerBg,
    };
    final Ico? icon = switch (a.state) {
      AssignState.queued || AssignState.waiting => Ico.hourglass,
      AssignState.done => Ico.check,
      _ => null,
    };
    final chip = Container(
      key: ValueKey('runner-chip-${a.taskId}'),
      padding: const EdgeInsets.fromLTRB(Space.sm, Space.xxs, Space.md, Space.xs),
      decoration: BoxDecoration(color: background, borderRadius: Radii.pill),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            Icon20(icon, size: IconSize.xs, color: c.textSecondary),
            const Gap(Space.xs),
          ],
          Flexible(
            child: Text(
              runnerChipText(a),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textPrimary, weight: Weight.strong),
            ),
          ),
        ],
      ),
    );
    final reason = a.reason;
    final progress = runnerProgressText(a);
    if (reason == null && progress == null && !a.outdated) return chip;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        chip,
        if (reason != null) ...[
          const Gap(Space.xxs),
          Text(
            reason,
            style: ts(context.type.uiSmall, c.textSecondary, weight: Weight.regular),
          ),
        ],
        if (a.outdated) ...[
          const Gap(Space.xxs),
          Text(
            'Running on an earlier version of the note',
            key: ValueKey('runner-outdated-${a.taskId}'),
            style: ts(context.type.uiSmall, c.textSecondary, weight: Weight.regular),
          ),
        ],
        if (progress != null) ...[
          const Gap(Space.xxs),
          AppTooltip(
            label: progress,
            child: Text(
              progress,
              key: ValueKey('runner-progress-${a.taskId}'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(
                context.type.uiSmall,
                a.state == AssignState.running ? c.textSecondary : c.textTertiary,
               weight: Weight.regular),
            ),
          ),
        ],
      ],
    );
  }
}
