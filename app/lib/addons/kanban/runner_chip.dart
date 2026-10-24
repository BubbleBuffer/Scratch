// A card's runner chip, "Laptop · waiting", with the assignment's reason
// underneath when there is one. The chip stays on one line and ends in an
// ellipsis.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'runner_backend.dart';

/// The chip's words: the runner's name, a dot and the state.
String runnerChipText(RunnerAssignment assignment) =>
    '${assignment.runnerName} · ${assignment.state.label}';

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
      AssignState.waiting => c.attnBg,
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
      padding: const EdgeInsets.fromLTRB(8, 4, 10, 5),
      decoration: BoxDecoration(color: background, borderRadius: Radii.pill),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            Icon20(icon, size: 12, color: c.textSecondary),
            const SizedBox(width: 5),
          ],
          Flexible(
            child: Text(
              runnerChipText(a),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(const TypeSpec(12, 16, 600), c.textPrimary),
            ),
          ),
        ],
      ),
    );
    final reason = a.reason;
    if (reason == null) return chip;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        chip,
        const SizedBox(height: 4),
        Text(
          reason,
          style: ts(const TypeSpec(12.5, 17, 400), c.textSecondary),
        ),
      ],
    );
  }
}
