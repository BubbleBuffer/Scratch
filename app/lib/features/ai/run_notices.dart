// Phone notices for delegated runs. When the task rows the phone syncs show
// that a run finished or failed, the phone posts a local notification; a tap
// opens the task's note. There is no hub push and no background service: a
// notice appears when the app is running and its sync applies the change, or
// at the next start of the app.
//
// What counts as a notice is decided in one place, [runNoticeOf]. Everything
// else here (what was noticed before, who made the change, how it is posted)
// does not depend on the run states, so a new state needs a line there only.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../core/clock.dart';
import '../../core/paths.dart';
import '../reminders/backends.dart';
import '../settings/settings_store.dart';
import 'ai_store.dart';

/// What a run's change says to the user.
enum RunNoticeKind {
  finished('Task finished'),
  review('Task ready for review'),
  failed('Task failed');

  const RunNoticeKind(this.title);

  /// The notification title, before the task's name.
  final String title;
}

/// The notice that row [t] calls for, or null when it calls for none (queued,
/// running, cancelled by the user, or a state this does not know). The one
/// place that reads run states: the run states that need attention (blocked,
/// moved to Review) are added here.
RunNoticeKind? runNoticeOf(AiTask t) => switch (t.status) {
  AiStatus.done => RunNoticeKind.finished,
  AiStatus.review => RunNoticeKind.review,
  AiStatus.failed || AiStatus.interrupted => RunNoticeKind.failed,
  _ => null,
};

/// Names the attempt state that was noticed, so the same one is never noticed
/// twice but a retry that ends again is. Today the state is the status; an
/// attempt number goes here when rows carry one.
String runNoticeKey(AiTask t) => t.status;

/// What a look at the task rows decided.
class RunNoticePlan {
  const RunNoticePlan({required this.notify, required this.seen});

  /// The rows to post a notice for.
  final List<AiTask> notify;

  /// The handled states to keep, task id to [runNoticeKey].
  final Map<String, String> seen;
}

/// Decides which rows get a notice. [seen] is what was handled before (null:
/// this is the first look, which records the rows and notices none). A row is
/// noticed when it is in a noticeable state not yet in [seen], unless this
/// device made the change ([deviceId] ran it) or the row last changed before
/// [sinceMs], so rows that sync in later from before the first look (a new
/// install's history) stay quiet. A row that is back to running or queued is
/// forgotten, so its next end is noticed again.
RunNoticePlan planRunNotices({
  required List<AiTask> tasks,
  required Map<String, String>? seen,
  required int sinceMs,
  required String deviceId,
}) {
  final next = <String, String>{};
  final notify = <AiTask>[];
  for (final t in tasks) {
    if (runNoticeOf(t) == null) continue;
    final key = runNoticeKey(t);
    next[t.id] = key;
    if (seen == null || seen[t.id] == key) continue;
    if (t.device == deviceId || t.updatedMs < sinceMs) continue;
    notify.add(t);
  }
  return RunNoticePlan(notify: notify, seen: next);
}

/// A notification id for task [id], so a second notice for it replaces the
/// first.
int runNoticeId(String id) {
  var hash = 0x811c9dc5;
  for (final byte in utf8.encode('run|$id')) {
    hash = ((hash ^ byte) * 0x01000193) & 0xffffffff;
  }
  final positive = hash & 0x7fffffff;
  return positive == 0 ? 1 : positive;
}

/// The notification body for a notice: a failed run's reason, one line.
String runNoticeBody(RunNoticeKind kind, AiTask t) {
  if (kind != RunNoticeKind.failed) return '';
  final line = t.error.trim().split('\n').first.trim();
  return line.length <= 140 ? line : '${line.substring(0, 139)}…';
}

/// Posts run notices from the AI store's task rows. One per app state; it
/// does nothing off Android.
class RunNotifier {
  RunNotifier({
    required this.deviceId,
    required this.titleOf,
    required this.settings,
    required this.backend,
    required this.source,
    bool? enabledHere,
  }) : _enabledHere =
           enabledHere ??
           (!kIsWeb && defaultTargetPlatform == TargetPlatform.android);

  /// This device's id; rows it ran itself get no notice.
  final String deviceId;

  /// The title of note `page`, or null when it is gone or in the trash.
  final Future<String?> Function(String page) titleOf;
  final AppSettings settings;
  final ReminderBackend backend;
  final AiTaskSource source;
  final bool _enabledHere;

  bool _started = false, _disposed = false, _busy = false, _again = false;

  /// Starts watching the rows. The first look records the rows already there.
  Future<void> start() async {
    if (_started || !_enabledHere) return;
    _started = true;
    source.addListener(_changed);
    source.onDelegated = _delegated;
    await _look();
  }

  /// The user turned the setting on: ask for the permission now.
  Future<void> enable() async {
    if (!_enabledHere) return;
    await backend.ensurePermission();
  }

  void _delegated() {
    if (settings.runNotices) unawaited(backend.ensurePermission());
  }

  void _changed() => unawaited(_look());

  /// One look at the rows; looks that arrive during one run after it.
  Future<void> _look() async {
    if (_busy) {
      _again = true;
      return;
    }
    _busy = true;
    try {
      do {
        _again = false;
        await _lookOnce();
      } while (_again && !_disposed);
    } catch (e) {
      AppLog.info('run notices failed: $e');
    } finally {
      _busy = false;
    }
  }

  Future<void> _lookOnce() async {
    if (_disposed) return;
    final old = settings.runNoticesSeen;
    final since =
        settings.runNoticesSince ?? clockNow().millisecondsSinceEpoch;
    final plan = planRunNotices(
      tasks: source.tasks,
      seen: old,
      sinceMs: since,
      deviceId: deviceId,
    );
    // Kept before posting: a failed post is not retried into a repeat.
    if (old == null || !mapEquals(old, plan.seen)) {
      settings.update((s) {
        s.runNoticesSeen = plan.seen;
        s.runNoticesSince ??= since;
      });
    }
    if (!settings.runNotices || plan.notify.isEmpty) return;
    if (!await backend.ensurePermission()) return;
    for (final t in plan.notify) {
      final kind = runNoticeOf(t);
      final page = t.page;
      if (kind == null || page == null) continue;
      final title = await titleOf(page);
      if (title == null) continue;
      await backend.showNow(
        id: runNoticeId(t.id),
        title: '${kind.title}: $title',
        body: runNoticeBody(kind, t),
        payload: jsonEncode({'page': page}),
      );
    }
  }

  void dispose() {
    _disposed = true;
    if (_started) {
      source.removeListener(_changed);
      if (source.onDelegated == _delegated) source.onDelegated = null;
    }
  }
}
