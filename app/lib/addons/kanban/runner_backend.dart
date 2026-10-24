// The runner picker's view of runners and assignments. The UI talks only to
// [RunnerBackend]. This branch ships only the fake (runner_fake.dart); the
// real backend answers from the AI task rows later. Nothing here calls the
// Rust bridge or the hub.

import 'package:flutter/widgets.dart';

/// The hub, or a desktop that can run an agent for a task.
enum RunnerKind { hub, pc }

/// One runner as the picker lists it.
class Runner {
  const Runner({
    required this.id,
    required this.name,
    required this.kind,
    required this.online,
    this.lastSeen,
    this.isThisDevice = false,
    this.isDefault = false,
  });

  final String id;
  final String name;
  final RunnerKind kind;
  final bool online;

  /// When the runner was last seen, local time. Null for the hub.
  final DateTime? lastSeen;

  /// True for the desktop the app runs on.
  final bool isThisDevice;

  /// True for the runner the user chose as the default in Settings.
  final bool isDefault;
}

/// Where an assignment stands, as the card's chip words it.
enum AssignState {
  queued('queued'),
  waiting('waiting'),
  running('running'),
  cancelling('cancelling'),
  done('done'),
  failed('failed'),
  cancelled('cancelled');

  const AssignState(this.label);

  /// The word after the runner's name on the chip.
  final String label;
}

/// A task's assignment to a runner. [reason] is shown under the chip when
/// there is one, for example why the task waits.
class RunnerAssignment {
  const RunnerAssignment({
    required this.taskId,
    required this.runnerId,
    required this.runnerName,
    required this.state,
    this.reason,
  });

  final String taskId;
  final String runnerId;
  final String runnerName;
  final AssignState state;
  final String? reason;
}

/// The backend could not answer. [reason] is shown as is.
class RunnerUnavailable implements Exception {
  const RunnerUnavailable(this.reason);
  final String reason;

  @override
  String toString() => reason;
}

/// Lists the runners and assigns a task to one of them. An offline runner
/// accepts the task and answers `waiting` with the reason.
abstract class RunnerBackend {
  /// The hub first, then each PC, in the order the picker shows them.
  Future<List<Runner>> listRunners();

  /// Assigns the task [taskId] to the runner [runnerId].
  Future<RunnerAssignment> assign(String taskId, String runnerId);

  /// The assignment this backend already knows for [taskId], or null. Chips
  /// read it on every redraw, so it must not call the backend.
  RunnerAssignment? current(String taskId) => null;

  /// Fires when [current] may have changed. Null: it changes only through
  /// [RunnerAssignments.record].
  Listenable? get changes => null;
}

/// The assignments the UI has seen, per task id, and the backend that makes
/// them. Cards listen here, so a new assignment redraws its chip.
class RunnerAssignments extends ChangeNotifier {
  RunnerAssignments(this.backend) {
    backend.changes?.addListener(notifyListeners);
  }

  final RunnerBackend backend;
  final Map<String, RunnerAssignment> _byTask = {};

  /// The task's latest assignment, or null when it has none. The backend's
  /// own answer wins over one recorded from an earlier assign.
  RunnerAssignment? of(String taskId) =>
      backend.current(taskId) ?? _byTask[taskId];

  Future<List<Runner>> runners() => backend.listRunners();

  /// Assigns [taskId] to [runnerId] and keeps the answer.
  Future<RunnerAssignment> assign(String taskId, String runnerId) async {
    final assignment = await backend.assign(taskId, runnerId);
    record(assignment);
    return assignment;
  }

  /// Keeps [assignment] as the task's state and tells the cards.
  void record(RunnerAssignment assignment) {
    _byTask[assignment.taskId] = assignment;
    notifyListeners();
  }

  @override
  void dispose() {
    backend.changes?.removeListener(notifyListeners);
    super.dispose();
  }
}

/// How long ago [seen] was, as the picker words it: "2 h ago".
String lastSeenText(DateTime seen, DateTime now) {
  final gap = now.difference(seen);
  if (gap.inMinutes < 1) return 'just now';
  if (gap.inHours < 1) return '${gap.inMinutes} min ago';
  if (gap.inDays < 1) return '${gap.inHours} h ago';
  return '${gap.inDays} d ago';
}

/// Gives the board its runner assignments. Without this ancestor the board
/// offers no runner picker, unless it runs as the demo.
class RunnerScope extends InheritedNotifier<RunnerAssignments> {
  const RunnerScope({
    super.key,
    required RunnerAssignments assignments,
    required super.child,
  }) : super(notifier: assignments);

  static RunnerAssignments? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<RunnerScope>()
      ?.notifier;
}

/// True when the app runs with `--dart-define=SEED_DEMO=true`. The board then
/// uses the fake runners from runner_fake.dart when no scope is given.
const runnerSeedDemo = bool.fromEnvironment('SEED_DEMO');
