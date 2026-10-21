// The live state of one search query: its watch on the core, the ordered
// hits, the row facts (icon, status, task state) and the optimistic ticks.
// A search card owns one, and so does a chip's results panel (Form B). The
// chip shares its watch with its panel: one watch per visible chip.
//
// Task rows: a task's status lives in the Tasks add-on's type data, which the
// core indexes as the `status` property (queries such as `-status:done` use
// it). This reads and writes `status` there itself, the same write the Tasks
// note uses, and does not import Tasks. A tick hides its row until the next
// live result says whether the row left the list.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../core/vault.dart';
import '../../src/rust/api/types.dart';
import '../../src/rust/api/vault.dart';
import 'query_hits.dart';
import 'query_status.dart';
import 'query_ticks.dart';
import 'query_wording.dart';

/// Results requested from the core per watch. Cards show a few rows and
/// expand in place; the chip panel shows the same rows.
const queryWatchLimit = 50;

/// The Tasks note type's id and add-on id, as literals: this add-on does not
/// import Tasks. Task status is the `status` key of the Tasks type data.
const _taskPageType = 'task';
const _tasksAddon = 'tasks';

/// The two bridge calls a watch makes. Tests replace [queryBridge] to count
/// the live handles; the core exposes no count of its own.
class QueryBridge {
  const QueryBridge();

  Stream<LiveQueryUpdate> watch(NotesVault raw, String query, int limit) =>
      raw.watchQuery(query: query, limit: limit);

  Future<void> unwatch(NotesVault raw, BigInt handle) =>
      raw.unwatchQuery(handle: handle);
}

QueryBridge queryBridge = const QueryBridge();

/// What a row shows beyond its hit, read from the page. [epoch] is the
/// update it was read for; an older one is read again.
class QueryRowFacts {
  const QueryRowFacts({
    required this.epoch,
    this.icon,
    this.statusLabel,
    this.done = false,
  });

  final int epoch;
  final String? icon;
  final String? statusLabel;
  final bool done;
}

/// One query's live results. Notifies its listeners when the hits, the
/// count, the wording or a row's facts change. Dispose it to release the
/// core handle.
class QueryWatch extends ChangeNotifier {
  QueryWatch({
    required this.vault,
    required this.tasksOn,
    required String query,
  }) {
    _query = query;
  }

  final Vault vault;

  /// Whether the Tasks add-on is on. Off means task rows have no checkbox.
  final bool tasksOn;

  late String _query;
  QueryHitList _list = QueryHitList();
  ParsedQuery? _parsed;
  StreamSubscription<LiveQueryUpdate>? _sub;
  BigInt? _handle;
  int _generation = 0;
  int _infoEpoch = 0;
  bool _ready = false;
  bool _disposed = false;
  Object? _error;

  /// Ticks hidden until the core's next result (see [settlePendingTicks]).
  final _pendingTicks = <HitKey, PendingTick>{};
  final _infos = <HitKey, QueryRowFacts>{};

  /// Row reads in flight, by the update epoch they were started for.
  final _loadingInfo = <HitKey, int>{};

  /// Task status by page id, as last read or written here (true = done).
  final _doneById = <String, bool>{};

  final _rowsLeft = StreamController<Hit>.broadcast();

  /// The row a settled tick left out of the list, once per such row.
  Stream<Hit> get rowsLeft => _rowsLeft.stream;

  /// Called when a tick's status write fails.
  void Function(Object error)? onWriteFailed;

  String get query => _query;
  List<Hit> get hits => _list.hits;
  ParsedQuery? get parsed => _parsed;
  bool get ready => _ready;
  Object? get error => _error;

  /// The core's total less the rows hidden by a pending tick, which the core
  /// still counts until its answer arrives.
  int get count => _list.total - hiddenHits.length;

  List<Hit> get hiddenHits => [
    for (final hit in _list.hits)
      if (_pendingTicks.containsKey(keyOfHit(hit))) hit,
  ];

  List<Hit> get visibleHits => [
    for (final hit in _list.hits)
      if (!_pendingTicks.containsKey(keyOfHit(hit))) hit,
  ];

  /// The wording of the current count (chip label and empty line).
  QueryWording get wording => queryWording(
    query: _query,
    tokens: _parsed?.tokens ?? const [],
    count: count,
  );

  /// Starts the watch and reads the query's tokens. Call once, after
  /// construction.
  void start() {
    _watch(_query);
    _parse(_query);
  }

  /// Moves to [query]: the old watch is released and a new one starts. The
  /// rows stay out until the new watch's first result.
  void setQuery(String query) {
    if (query == _query) return;
    _query = query;
    _watch(query);
    _parse(query);
    _notify();
  }

  /// Whether [hit] is a task row that can be ticked.
  bool isTaskRow(Hit hit) =>
      tasksOn && hit.pageType == _taskPageType && hit.source == 'vault';

  /// Whether the task is done: as last written here, else as read from the
  /// page. Null-safe: a row whose facts are not read yet is not done.
  bool isDone(Hit hit) =>
      _doneById[hit.id] ?? _infos[keyOfHit(hit)]?.done ?? false;

  /// The row's facts, or null while they are read. A stale read is replaced.
  QueryRowFacts? infoOf(Hit hit) {
    final key = keyOfHit(hit);
    final info = _infos[key];
    if ((info == null || info.epoch != _infoEpoch) &&
        _loadingInfo[key] != _infoEpoch) {
      _loadingInfo[key] = _infoEpoch;
      unawaited(_loadRow(hit, _infoEpoch));
    }
    return info;
  }

  /// Writes the task's status, then resyncs the watch so the core's answer
  /// arrives. A tick (done: true) hides the row until that answer.
  Future<void> setDone(Hit hit, {required bool done}) async {
    final key = keyOfHit(hit);
    if (done) {
      _pendingTicks[key] = PendingTick(hit);
      _notify();
    }
    try {
      await vault.typeDataSet(
        hit.id,
        _tasksAddon,
        const ['status'],
        jsonEncode(done ? 'done' : 'todo'),
      );
      await vault.flush();
      _doneById[hit.id] = done;
    } catch (error) {
      if (_disposed) return;
      _pendingTicks.remove(key);
      _notify();
      onWriteFailed?.call(error);
      return;
    }
    if (_disposed) return;
    _pendingTicks[key]?.writing = false;
    _watch(_query, resync: true);
  }

  /// Ticks or unticks [hit] (the checkbox).
  void toggleDone(Hit hit) => setDone(hit, done: !isDone(hit));

  @override
  void dispose() {
    _disposed = true;
    _unwatch();
    _rowsLeft.close();
    super.dispose();
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  /// Test seam: reports [hit] as having left the list, as a settled tick does.
  @visibleForTesting
  void reportLeftForTest(Hit hit) => _rowsLeft.add(hit);

  // ---- the live watch ----

  /// Starts a watch on [query]. A resync keeps the rows on screen until the
  /// new watch's first result replaces them.
  void _watch(String query, {bool resync = false}) {
    _unwatch();
    if (!resync) {
      _list = QueryHitList();
      _ready = false;
      _error = null;
      _pendingTicks.clear();
    }
    if (query.trim().isEmpty) return;
    final raw = vault.raw;
    final generation = _generation;
    final stream = queryBridge.watch(raw, query, queryWatchLimit);
    _sub = stream.listen(
      (update) {
        // A stale update (its watch changed query or was disposed) only
        // releases its registration.
        if (generation != _generation || _disposed) {
          _release(raw, update.handle);
          return;
        }
        _handle = update.handle;
        _list.apply(update);
        _ready = true;
        _infoEpoch++;
        final left = settlePendingTicks(_pendingTicks, _list.hits);
        _notify();
        if (left.isNotEmpty && !_rowsLeft.isClosed) {
          _rowsLeft.add(left.last);
        }
      },
      onError: (Object error) {
        if (generation != _generation || _disposed) return;
        _error = error;
        _notify();
      },
    );
  }

  /// Ends the current watch. The handle is released before the stream is
  /// cancelled, so an idle stream finishes too.
  void _unwatch() {
    _generation++;
    final handle = _handle;
    final sub = _sub;
    _handle = null;
    _sub = null;
    if (handle != null) _release(vault.raw, handle);
    sub?.cancel();
  }

  /// Fire-and-forget: dispose cannot await. The vault may already be closed,
  /// in which case the core has dropped the registration anyway.
  void _release(NotesVault raw, BigInt handle) {
    unawaited(queryBridge.unwatch(raw, handle).catchError((Object _) {}));
  }

  Future<void> _parse(String query) async {
    try {
      final parsed = await vault.run((v) => v.parseSearch(q: query));
      if (!_disposed && query == _query) {
        _parsed = parsed;
        _notify();
      }
    } catch (_) {
      // The wording falls back to the generic one.
    }
  }

  Future<void> _loadRow(Hit hit, int epoch) async {
    final key = keyOfHit(hit);
    try {
      final QueryRowFacts info;
      if (hit.source != 'vault') {
        info = QueryRowFacts(epoch: epoch);
      } else {
        final page = await vault.loadPage(hit.id);
        final parent = page.parent;
        final schema = parent == null
            ? const <PropDef>[]
            : await vault.run((v) => v.getSchema(collection: parent));
        final isTask = tasksOn && hit.pageType == _taskPageType;
        if (isTask) _doneById[hit.id] = await _readDone(hit.id);
        info = QueryRowFacts(
          epoch: epoch,
          icon: page.icon,
          statusLabel: isTask ? null : statusLabelOf(schema, page.properties),
          done: _doneById[hit.id] ?? false,
        );
      }
      if (_disposed || epoch != _infoEpoch) return;
      _infos[key] = info;
      _notify();
    } catch (_) {
      // The row keeps the generic icon when its page cannot be read.
    } finally {
      if (_loadingInfo[key] == epoch) _loadingInfo.remove(key);
    }
  }

  Future<bool> _readDone(String page) async {
    final value = jsonDecode(await vault.typeData(page, _tasksAddon));
    return value is Map && value['status'] == 'done';
  }
}
