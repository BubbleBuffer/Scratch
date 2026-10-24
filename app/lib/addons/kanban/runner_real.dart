// The real runner backend, on the open vault's agent. The runners are the
// vault's (ai_list_runners, hub first). An assign queues a real AI task row
// pinned to the runner, even an offline one. A card's state is read from the
// newest task row about the card's page. Changes arrive through the AI store's
// notifications, so nothing here polls.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart'
    show AiTask, AiTaskSource;
import '../../src/rust/api/ai.dart' show AiRunner, AiRunnerKind, AiRunnerList;
import 'runner_backend.dart';

/// What an assigned card asks the agent to do with its note.
const runnerCardPrompt = 'Work on this note and write the result at its end.';

class RealRunnerBackend extends ChangeNotifier implements RunnerBackend {
  RealRunnerBackend(this.source, {DateTime Function()? clock})
    : _clock = clock ?? DateTime.now {
    source.addListener(_onSource);
  }

  final AiTaskSource source;
  final DateTime Function() _clock;

  /// The runners by id from the last listing; null until the first one.
  Map<String, AiRunner>? _runners;
  bool _listing = false;
  bool _disposed = false;

  @override
  Listenable get changes => this;

  @override
  void dispose() {
    _disposed = true;
    source.removeListener(_onSource);
    super.dispose();
  }

  /// The rows or the store changed. A PC may also have come or gone, so the
  /// runners are listed again. A listing writes nothing, so this cannot loop.
  void _onSource() {
    notifyListeners();
    unawaited(_refreshRunners());
  }

  /// Lists the runners into the cache and tells the cards. Throws when the
  /// vault cannot list them.
  Future<AiRunnerList> _fetch() async {
    final list = await source.runnerBridge.listRunners(
      hubUsable: source.hubUsable,
    );
    _runners = {for (final r in list.runners) r.id: r};
    if (!_disposed) notifyListeners();
    return list;
  }

  /// A background listing: one at a time, and it never throws. After a
  /// failure the cache is empty, so the cards do not ask again on every draw.
  Future<void> _refreshRunners() async {
    if (_listing || _disposed) return;
    _listing = true;
    try {
      await _fetch();
    } catch (_) {
      _runners ??= {};
    } finally {
      _listing = false;
    }
  }

  @override
  RunnerAssignment? current(String taskId) {
    if (_runners == null) unawaited(_refreshRunners());
    for (final t in source.tasks) {
      if (t.page == taskId) return _fromTask(taskId, t);
    }
    return null;
  }

  @override
  Future<List<Runner>> listRunners() async {
    try {
      final list = await _fetch();
      return [for (final r in list.runners) _runner(r, list.defaultRunnerId)];
    } catch (e) {
      throw RunnerUnavailable('$e');
    }
  }

  @override
  Future<RunnerAssignment> assign(String taskId, String runnerId) async {
    try {
      if (_runners?.containsKey(runnerId) != true) await _fetch();
      final task = await source.runnerBridge.assign(
        prompt: runnerCardPrompt,
        page: taskId,
        runnerId: runnerId,
      );
      await source.refresh();
      return _fromTask(taskId, task) ??
          RunnerAssignment(
            taskId: taskId,
            runnerId: runnerId,
            runnerName: _nameOf(runnerId),
            state: AssignState.queued,
          );
    } catch (e) {
      throw RunnerUnavailable('$e');
    }
  }

  Runner _runner(AiRunner r, String defaultId) => Runner(
    id: r.id,
    name: r.name,
    kind: r.kind == AiRunnerKind.hub ? RunnerKind.hub : RunnerKind.pc,
    online: r.online,
    lastSeen: _time(r.lastSeenMs?.toInt()),
    isThisDevice: r.isThisDevice,
    isDefault: r.id == defaultId,
  );

  String _nameOf(String runnerId) => _runners?[runnerId]?.name ?? runnerId;

  DateTime? _time(int? ms) =>
      ms == null ? null : DateTime.fromMillisecondsSinceEpoch(ms);

  /// The chip state of [t] for the card [cardId]. A row without a runner
  /// (a legacy row) is not an assignment, so it shows nothing.
  RunnerAssignment? _fromTask(String cardId, AiTask t) {
    if (t.node.isEmpty) return null;
    final runner = _runners?[t.node];
    final s = _stateOf(t, runner);
    return RunnerAssignment(
      taskId: cardId,
      runnerId: t.node,
      runnerName: runner?.name ?? t.node,
      state: s.state,
      reason: s.reason,
    );
  }

  /// Maps a row's status to the chip. The bridge has no `cancelling` state,
  /// so a stopping run shows as running until it ends. An interrupted run
  /// reads as failed. A queued row pinned to an offline runner is waiting.
  ({AssignState state, String? reason}) _stateOf(AiTask t, AiRunner? runner) {
    switch (t.status) {
      case 'running':
        return (state: AssignState.running, reason: null);
      case 'done':
        return (state: AssignState.done, reason: null);
      case 'failed':
        return (
          state: AssignState.failed,
          reason: t.error.isEmpty ? null : t.error,
        );
      case 'interrupted':
        return (
          state: AssignState.failed,
          reason: 'The run was interrupted before it finished.',
        );
      case 'cancelled':
        return (state: AssignState.cancelled, reason: null);
      default:
        if (runner != null && !runner.online) {
          return (state: AssignState.waiting, reason: _offlineReason(runner));
        }
        return (state: AssignState.queued, reason: null);
    }
  }

  String _offlineReason(AiRunner runner) {
    final seen = _time(runner.lastSeenMs?.toInt());
    final since = seen == null
        ? ''
        : ' (last seen ${lastSeenText(seen, _clock())})';
    return '${runner.name} is offline$since';
  }
}

final _assignments = Expando<RunnerAssignments>('ai runner assignments');

/// The runner assignments of [source]'s agent: one per store, so every board
/// shares the chips and the picker's answers.
RunnerAssignments aiRunnerAssignments(AiTaskSource source) =>
    _assignments[source] ??= RunnerAssignments(RealRunnerBackend(source));
