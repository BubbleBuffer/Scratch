// Open pages. Each open page has a session: its edit queue, its undo stack,
// and a watch on the change feed. A change to the page that did not come
// from this session (sync, another window, restore) is loaded and handed to
// the editor through [PageSession.external]; a deleted page raises
// [PageSession.deleted].

import '../core/debounce.dart';

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/models.dart';
import '../core/vault.dart';
import 'op_queue.dart';
import 'undo.dart';

class PageSession {
  PageSession._(this.store, this.pageId) : queue = OpQueue(store.vault, pageId);

  final PageStore store;
  final String pageId;
  final OpQueue queue;
  final UndoStack undo = UndoStack();

  /// Content that changed outside this session, for the editor to adopt.
  final external = ValueNotifier<PageData?>(null);

  /// The page went to the trash or was purged.
  final deleted = ValueNotifier<bool>(false);

  /// The editor's current blocks and title, to tell an outside change from
  /// an echo of this session's own writes.
  List<FlatBlock> Function()? currentBlocks;
  String Function()? currentTitle;

  int _refs = 0;
  bool _detached = false;
  bool _checking = false;
  bool _recheck = false;
  Future<void> _rename = Future.value();

  Vault get vault => store.vault;

  Future<PageData> load() => vault.loadPage(pageId);

  /// Queues block ops for `vault.apply` (in order, coalescing typing).
  void apply(List<BOp> ops) => queue.add(ops);

  void rename(String title) {
    queue.generation++;
    _renames++;
    _rename = _rename.then((_) => vault.renamePage(pageId, title)).catchError((Object e) {
      debugPrint('rename failed: $e');
    }).whenComplete(() => _renames--);
  }

  int _renames = 0;

  /// Waits for queued edits and the title, then flushes to disk.
  Future<void> save() async {
    await Future.wait([queue.drain(), _rename]);
    await vault.flush();
  }

  Future<void> _checkExternal() async {
    if (_closed || vault.isClosed) return;
    if (_checking) {
      _recheck = true;
      return;
    }
    _checking = true;
    try {
      do {
        _recheck = false;
        if (!queue.idle) continue;
        final gen = queue.generation;
        final PageData p;
        try {
          p = await load();
        } catch (_) {
          continue;
        }
        if (_closed || vault.isClosed || gen != queue.generation || !queue.idle || _renames > 0) continue;
        if (p.inTrash) {
          deleted.value = true;
          return;
        }
        final blocks = currentBlocks?.call();
        final title = currentTitle?.call();
        if (blocks == null) continue;
        if (!sameBlocks(blocks, p.blocks) || (title != null && title != p.title)) {
          external.value = p;
        }
      } while (_recheck && !_closed && !vault.isClosed);
    } finally {
      _checking = false;
    }
  }

  /// A change arrived. Right after this session's own writes it is most
  /// likely their echo, so the check waits until typing pauses.
  void _changed() {
    if (_closed || vault.isClosed) return;
    if (queue.generation != _seenGeneration) {
      _seenGeneration = queue.generation;
      _deferred?.cancel();
      _deferred = Timer(Debounce.testDelay(settleDelay), () {
        _deferred = null;
        _checkExternal();
      });
      return;
    }
    if (_deferred == null) _checkExternal();
  }

  static const settleDelay = Duration(milliseconds: 1500);
  int _seenGeneration = 0;
  Timer? _deferred;

  bool _closed = false;

  void _cancelChecks() {
    _deferred?.cancel();
    _deferred = null;
    _recheck = false;
  }

  void _dispose() {
    _closed = true;
    _cancelChecks();
    external.dispose();
    deleted.dispose();
    undo.dispose();
  }
}

/// Same blocks in the same order with the same content.
bool sameBlocks(List<FlatBlock> a, List<FlatBlock> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    final x = a[i], y = b[i];
    if (x.id != y.id ||
        x.kind != y.kind ||
        x.text != y.text ||
        x.checked != y.checked ||
        x.depth != y.depth ||
        x.target != y.target ||
        x.color != y.color ||
        x.width != y.width ||
        x.marks.length != y.marks.length) {
      return false;
    }
    for (var j = 0; j < x.marks.length; j++) {
      if (x.marks[j] != y.marks[j]) return false;
    }
  }
  return true;
}

class PageStore extends ChangeNotifier {
  PageStore(this.vault, this.feed) {
    feed.addListener(_onChanges);
    vault.addCloseListener(_onVaultClosed);
  }

  final Vault vault;
  final ChangeFeed feed;
  final Map<String, PageSession> _sessions = {};

  /// Completes when the last closed page has written everything.
  Future<void> lastSave = Future.value();

  PageSession? session(String id) => _sessions[id];

  /// Opens (or shares) the session for [id]. Pair with [close].
  PageSession open(String id) {
    final s = _sessions.putIfAbsent(id, () => PageSession._(this, id));
    s._refs++;
    return s;
  }

  /// A session for showing a past version: not shared, never watched or
  /// saved. Pair with [close].
  PageSession detached(String id) => PageSession._(this, id)
    .._detached = true
    .._refs = 1;

  /// Releases [s]; the last release saves and drops it.
  void close(PageSession s) {
    if (--s._refs > 0) return;
    if (s._detached) {
      s._dispose();
      return;
    }
    _sessions.remove(s.pageId);
    s._deferred?.cancel();
    final save = s.save().catchError((Object e) => debugPrint('save failed: $e'));
    lastSave = Future.wait([lastSave, save]).then((_) {});
    save.whenComplete(s._dispose);
  }

  /// Moves [id] (and its subtree) to the trash.
  Future<void> trash(String id) async {
    final s = _sessions[id];
    if (s != null) await s.queue.drain();
    await vault.deletePage(id);
    s?.deleted.value = true;
  }

  void _onChanges(ChangeBatch b) {
    if (vault.isClosed) return;
    for (final s in _sessions.values) {
      if (b.touches(s.pageId)) {
        final gone = b.changes.any((c) => c.page == s.pageId && c.kind == ChangeKind.pageDeleted);
        if (gone) {
          s.deleted.value = true;
        } else {
          s._changed();
        }
      } else if (b.treeChanged) {
        // Moves to the trash arrive as tree changes.
        s._changed();
      }
    }
  }

  void _onVaultClosed() {
    for (final session in _sessions.values) {
      session._cancelChecks();
    }
  }

  @override
  void dispose() {
    vault.removeCloseListener(_onVaultClosed);
    _onVaultClosed();
    feed.removeListener(_onChanges);
    super.dispose();
  }
}
