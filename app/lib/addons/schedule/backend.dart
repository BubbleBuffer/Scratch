// The schedule add-on's view of the engine. The UI talks only to this
// interface. The engine implementation (engine.dart) calls the vault's local
// schedule calls for this device's runner and the hub's agent connection for a
// hub runner; the fake (fake.dart) stands in for tests.
// Hashes, titles and next runs always come from the engine.

import 'package:flutter/widgets.dart';

/// The engine could not answer: the hub is not connected or not reachable,
/// or the runner is one this build cannot call. [reason] is shown as is.
class ScheduleUnavailable implements Exception {
  const ScheduleUnavailable(this.reason);
  final String reason;

  @override
  String toString() => reason;
}

/// The result of checking a definition: whether it parses, the hash to accept
/// it with, its next runs and the first occurrence's rendered title. [ok]
/// false means the definition is invalid and [reason] says why.
class SchedulePreview {
  const SchedulePreview({
    required this.ok,
    this.reason,
    this.currentHash,
    this.nextRuns = const [],
    this.renderedTitle = '',
  });

  final bool ok;
  final String? reason;

  /// The hash of runner, tools and steps. Pass it to [ScheduleBackend.accept].
  final String? currentHash;

  /// The next occurrences, local time (at most three).
  final List<DateTime> nextRuns;

  /// The first step's title with the date fields filled in by the engine.
  final String renderedTitle;
}

/// One occurrence's outcome as the runner reports it.
class ScheduleRunRecord {
  const ScheduleRunRecord({
    this.occurrence,
    this.at,
    required this.result,
    this.page,
    this.error,
  });

  /// The occurrence time this run was for (its stable id part), local time.
  final DateTime? occurrence;

  /// When the runner finished it, local time.
  final DateTime? at;

  /// `created`, `skipped` or `failed`.
  final String result;

  /// The created page, when there is one.
  final String? page;
  final String? error;
}

/// The runner's live state for one schedule, as the runner answers now.
class ScheduleLiveState {
  const ScheduleLiveState({
    required this.accepted,
    required this.hashMatches,
    required this.paused,
    this.runnerState = 'ok',
    this.currentHash,
    this.lastRuns = const [],
    this.nextRun,
    this.reason,
  });

  final bool accepted;
  final bool hashMatches;
  final bool paused;

  /// The engine's state: `ok`, `needs_acceptance`, `paused`, `runner_off` or
  /// `refused`.
  final String runnerState;

  /// The hash of the definition the runner holds now.
  final String? currentHash;

  /// Oldest first, as the runner keeps them.
  final List<ScheduleRunRecord> lastRuns;
  final DateTime? nextRun;

  /// Why the runner refuses the schedule, when it does.
  final String? reason;
}

/// A runner node (a hub or a desktop) from the vault's node records.
class ScheduleNode {
  const ScheduleNode({
    required this.id,
    this.deviceId = '',
    required this.name,
    required this.kind,
    required this.available,
    this.lastSeen,
  });

  /// The record's page id.
  final String id;

  /// The device the record belongs to.
  final String deviceId;
  final String name;

  /// `hub` or `desktop`.
  final String kind;

  /// True while the node runs schedules (its device setting is on).
  final bool available;
  final DateTime? lastSeen;

  /// The kind as shown to the user.
  String get kindLabel => kind == 'hub' ? 'Hub' : 'Desktop';
}

/// The engine operations the schedule UI needs. Every runner call names the
/// runner's node record id.
abstract class ScheduleBackend {
  const ScheduleBackend();

  /// Checks [scheduleJson] (the definition area as JSON). Saves nothing.
  Future<SchedulePreview> preview(String scheduleJson);

  /// The live state of [pageId] on [runnerId].
  Future<ScheduleLiveState> state(String pageId, String runnerId);

  /// Accepts the definition whose [hash] the user reviewed, on [runnerId].
  Future<ScheduleLiveState> accept(String pageId, String runnerId, String hash);

  /// Pauses or resumes [pageId] on [runnerId].
  Future<ScheduleLiveState> pause(String pageId, String runnerId, bool paused);

  /// Runs [pageId] once now on [runnerId].
  Future<ScheduleRunRecord> runNow(String pageId, String runnerId);

  /// Every node record in the vault.
  Future<List<ScheduleNode>> listNodes();
}

/// Gives the schedule UI its backend. Without this ancestor the UI uses
/// [UnavailableScheduleBackend].
class ScheduleBackendScope extends InheritedWidget {
  const ScheduleBackendScope({
    super.key,
    required this.backend,
    required super.child,
  });

  final ScheduleBackend backend;

  static ScheduleBackend of(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<ScheduleBackendScope>()
          ?.backend ??
      const UnavailableScheduleBackend();

  @override
  bool updateShouldNotify(ScheduleBackendScope old) => old.backend != backend;
}

/// The backend used when no engine is scoped. Every call fails with a plain
/// reason, so the UI shows that nothing runs instead of pretending.
class UnavailableScheduleBackend extends ScheduleBackend {
  const UnavailableScheduleBackend();

  static const reason = 'The schedule engine is not connected.';

  @override
  Future<SchedulePreview> preview(String scheduleJson) async =>
      throw const ScheduleUnavailable(reason);

  @override
  Future<ScheduleLiveState> state(String pageId, String runnerId) async =>
      throw const ScheduleUnavailable(reason);

  @override
  Future<ScheduleLiveState> accept(
    String pageId,
    String runnerId,
    String hash,
  ) async => throw const ScheduleUnavailable(reason);

  @override
  Future<ScheduleLiveState> pause(
    String pageId,
    String runnerId,
    bool paused,
  ) async => throw const ScheduleUnavailable(reason);

  @override
  Future<ScheduleRunRecord> runNow(String pageId, String runnerId) async =>
      throw const ScheduleUnavailable(reason);

  @override
  Future<List<ScheduleNode>> listNodes() async => const [];
}
