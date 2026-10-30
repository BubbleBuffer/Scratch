// "Triage" on a project: choose the runner, then queue the run. The runner
// choice is a bottom sheet under 700 px and a centred panel at 700 px and
// wider. It lists the hub and each PC from the runner bridge, the default
// marked; one tap on a row starts the run.

import 'package:flutter/widgets.dart';

import '../../src/rust/api/ai.dart'
    show AiRunner, AiRunnerKind, AiRunnerList;
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../ai/ai_runners.dart';
import '../ai/ai_store.dart';
import 'triage_copy.dart';
import 'triage_store.dart';

const _sheetBelow = 700.0;

/// Starts Triage on the project (a top-level page) [projectId] titled
/// [projectTitle], on this app's AI store and triage store.
Future<void> startTriage(
  BuildContext context, {
  required String projectId,
  required String projectTitle,
}) {
  final ai = AiStore.of(context.app);
  return startTriageWith(
    context,
    bridge: TriageStore.of(context.app).bridge,
    runners: ai.runnerBridge,
    hubUsable: ai.hubUsable,
    projectId: projectId,
    projectTitle: projectTitle,
  );
}

/// [startTriage] with the bridges given (tests pass fakes).
Future<void> startTriageWith(
  BuildContext context, {
  required TriageBridge bridge,
  required AiRunnerBridge runners,
  required bool hubUsable,
  required String projectId,
  required String projectTitle,
}) async {
  final AiRunnerList list;
  try {
    list = await runners.listRunners(hubUsable: hubUsable);
  } catch (e) {
    if (context.mounted) showErrorToast(context, triageFailed(e));
    return;
  }
  if (!context.mounted) return;
  final runner = await showTriageRunnerChoice(
    context,
    list: list,
    projectTitle: projectTitle,
  );
  if (runner == null || !context.mounted) return;
  try {
    await bridge.start(projectId: projectId, runner: runner.id);
    if (context.mounted) showToast(context, triageStartedToast(runner.name));
  } catch (e) {
    if (context.mounted) showErrorToast(context, triageFailed(e));
  }
}

/// Shows the runner choice. Completes with the runner tapped, or null.
Future<AiRunner?> showTriageRunnerChoice(
  BuildContext context, {
  required AiRunnerList list,
  required String projectTitle,
}) {
  if (MediaQuery.sizeOf(context).width < _sheetBelow) {
    return showSheet<AiRunner>(
      context,
      (ctx) => SheetSurface(
        children: [
          SheetHeader(triageIntro(projectTitle)),
          for (final r in list.runners)
            SheetItem(
              key: ValueKey('triage-runner-${r.id}'),
              label: _label(r, list),
              selected: r.id == list.defaultRunnerId,
              onTap: () => Navigator.of(ctx).pop(r),
            ),
        ],
      ),
    );
  }
  return showModalPanel<AiRunner>(
    context,
    (ctx) => _RunnerChoice(
      list: list,
      projectTitle: projectTitle,
      onPick: (r) => Navigator.of(ctx).pop(r),
    ),
    width: 380,
    shrink: true,
  );
}

String _state(AiRunner r) => r.kind == AiRunnerKind.hub
    ? 'Hub'
    : r.online
    ? 'Online'
    : 'Offline: starts when it is back online';

String _label(AiRunner r, AiRunnerList list) =>
    '${r.name} · ${_state(r)}${r.id == list.defaultRunnerId ? ' · default' : ''}';

class _RunnerChoice extends StatelessWidget {
  const _RunnerChoice({
    required this.list,
    required this.projectTitle,
    required this.onPick,
  });

  final AiRunnerList list;
  final String projectTitle;
  final void Function(AiRunner) onPick;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return Padding(
      padding: const EdgeInsets.all(14),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Run Triage on', style: ts(ty.ui, c.textPrimary, weight: 600)),
          const SizedBox(height: 4),
          Text(
            triageIntro(projectTitle),
            style: ts(ty.uiSmall, c.textTertiary, weight: 400),
          ),
          const SizedBox(height: 8),
          for (final r in list.runners)
            PanelRow(
              key: ValueKey('triage-runner-${r.id}'),
              onTap: () => onPick(r),
              child: SizedBox(
                height: 44,
                child: Row(
                  children: [
                    Expanded(
                      child: Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            r.name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: ts(ty.ui, c.textPrimary, weight: 400),
                          ),
                          Text(
                            _state(r),
                            style: ts(ty.uiSmall, c.textTertiary, weight: 400),
                          ),
                        ],
                      ),
                    ),
                    if (r.id == list.defaultRunnerId)
                      Text(
                        'Default',
                        style: ts(ty.uiSmall, c.textTertiary, weight: 400),
                      ),
                  ],
                ),
              ),
            ),
          const SizedBox(height: 4),
        ],
      ),
    );
  }
}
