// Every page, most recently edited first (home's "Recents" and "All pages").
// Rows are fetched from the core in chunks as they scroll in, and the list
// patches content edits in place; structural changes reload (debounced).

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/vault.dart';

class PagesStore extends ChangeNotifier {
  PagesStore(this.vault, this.feed) {
    feed.addListener(_onChanges);
    vault.addCloseListener(_onVaultClosed);
  }

  final Vault vault;
  final ChangeFeed feed;

  static const chunk = 100;
  static const reloadDelay = Duration(milliseconds: 400);
  static const maxReloadRounds = 5;

  int count = 0;
  final Map<int, List<PageRow>> _chunks = {};
  final Set<int> _loading = {};
  // Keep positions from document scans too, without marking their chunks loaded.
  final Map<int, PageRow> _listed = {};
  final Set<String> _changed = {};
  final Set<String> _contentPending = {};
  bool _needsReload = false;
  int _changeVersion = 0;
  bool _refreshBusy = false;
  bool _refreshAgain = false;
  Future<void>? _reloadRequest;
  Future<bool>? _reloadInFlight;

  void _remember(int offset, List<PageRow> rows) {
    for (var i = 0; i < rows.length; i++) {
      _listed[offset + i] = rows[i];
    }
  }
  int _generation = 0;
  final _reload = Debounce();
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;

  /// Retries superseded snapshots up to [maxReloadRounds], then defers a full
  /// reload. Concurrent callers share the same request and latest retained state.
  Future<void> reload() {
    if (_inactive) return Future<void>.value();
    return _reloadRequest ??= _reloadUntilCurrent().whenComplete(() {
      _reloadRequest = null;
    });
  }

  Future<void> _reloadUntilCurrent() async {
    for (var round = 0; round < maxReloadRounds && !_inactive; round++) {
      await vault.flush();
      if (_inactive || await _reloadOnce()) return;
    }
    if (!_inactive) {
      _needsReload = true;
      _reload.run(reloadDelay, _refreshChanges);
    }
  }

  // Feed refreshes make one cancellable attempt; their supersession logic
  // schedules newer batches. Share reads with explicit reloads so their
  // document scans cannot run concurrently against the same mutable cursor.
  Future<bool> _reloadOnce() => _reloadInFlight ??= _loadPages().whenComplete(() {
    _reloadInFlight = null;
  });

  Future<bool> _loadPages() async {
    if (_inactive) return false;
    final gen = ++_generation;
    final n = await vault.pageCount();
    if (_inactive || gen != _generation) return false;
    final first = await vault.listPages(offset: 0, limit: chunk);
    if (_inactive) return false;
    await feed.poll();
    if (gen != _generation || _inactive) return false;
    count = n;
    _chunks
      ..clear()
      ..[0] = first;
    _loading.clear();
    _listed.clear();
    _remember(0, first);
    _isDatabase.clear();
    _resetDocs();
    // A full snapshot supersedes a cancelled incremental row patch. Keep
    // metadata stale so consumers can refresh it without the pending guard.
    _contentPending.clear();
    notifyListeners();
    await _scanDocs(gen);
    return !_inactive && gen == _generation;
  }

  /// Row [i], or null while its chunk loads (a notification follows).
  PageRow? rowAt(int i) {
    if (_inactive) return null;
    final c = i ~/ chunk;
    final rows = _chunks[c];
    if (rows != null) return i - c * chunk < rows.length ? rows[i - c * chunk] : null;
    if (_refreshBusy || _changed.isNotEmpty || _needsReload) return null;
    if (_loading.add(c)) {
      final gen = _generation;
      vault.listPages(offset: c * chunk, limit: chunk).then((rows) async {
        if (_inactive) return;
        await feed.poll();
        if (gen != _generation || _inactive) return;
        _chunks[c] = rows;
        _remember(c * chunk, rows);
        _loading.remove(c);
        notifyListeners();
      });
    }
    return null;
  }

  /// The first [n] rows that are loaded.
  List<PageRow> first(int n) => [
        for (var i = 0; i < n && i < count; i++) ?rowAt(i),
      ];

  // ---- documents: every page except database rows (home's "All pages") ----
  //
  // The core lists every page; rows of a collection are left out here. The
  // list is built by scanning `list_pages` in order, as far as the screen
  // needs, and rebuilt on reload. Whether a parent is a collection is cached
  // per page id.

  static const docScanChunk = 200;

  final List<PageRow> _docs = [];
  int _docScanned = 0;
  bool _docDone = false;
  bool _docScanning = false;
  int _docWanted = 60;
  final Map<String, bool> _isDatabase = {};

  /// Pages that are not database rows, as far as scanned.
  int get docCount => _docs.length;

  /// Whether [docCount] is final (every page was scanned).
  bool get docComplete => _docDone;

  /// Document [i], or null while the scan has not reached it (the scan goes
  /// on and a notification follows).
  PageRow? docAt(int i) {
    if (i < _docs.length) {
      if (i + 40 > _docs.length) _wantDocs(i + 60);
      return _docs[i];
    }
    _wantDocs(i + 60);
    return null;
  }

  void _wantDocs(int n) {
    if (_inactive) return;
    if (n > _docWanted) _docWanted = n;
    if (!_docDone && !_docScanning && !_refreshBusy && _changed.isEmpty && !_needsReload) {
      _scanDocs(_generation);
    }
  }

  /// Starts the scan over; the old list shows until the first new rows.
  void _resetDocs() {
    _docScanned = 0;
    _docDone = false;
    _docScanning = false;
  }

  Future<void> _scanDocs(int gen) async {
    if (_inactive || gen != _generation) return;
    _docScanning = true;
    try {
      while (!_inactive && gen == _generation && !_docDone && (_docScanned == 0 || _docs.length < _docWanted)) {
        final rows = await vault.listPages(offset: _docScanned, limit: docScanChunk);
        if (gen != _generation || _inactive) return;
        for (final r in rows) {
          if (r.isDatabase) _isDatabase[r.id] = true;
        }
        final parents = {for (final r in rows) ?r.parent}.where((p) => !_isDatabase.containsKey(p)).toList();
        for (final p in parents) {
          try {
            _isDatabase[p] = (await vault.loadPage(p)).isDatabase;
          } catch (_) {
            _isDatabase[p] = false;
          }
          if (gen != _generation || _inactive) return;
        }
        await feed.poll();
        if (gen != _generation || _inactive) return;
        _remember(_docScanned, rows);
        if (_docScanned == 0) _docs.clear();
        _docs.addAll(rows.where((r) => r.parent == null || _isDatabase[r.parent] != true));
        _docScanned += rows.length;
        if (rows.length < docScanChunk) _docDone = true;
        notifyListeners();
      }
    } catch (e) {
      debugPrint('list_pages failed: $e');
    } finally {
      if (gen == _generation) _docScanning = false;
    }
  }

  // ---- page metadata (title, icon, cover, parent) by id ----

  final Map<String, PageMeta> _meta = {};
  final Set<String> _metaLoading = {};
  final Map<String, int> _metaVersions = {};

  final Set<String> _metaStale = {};

  /// What links, mentions, breadcrumbs and home cards show for page [id]:
  /// cached (possibly stale while it reloads), or null while it first loads
  /// (a notification follows).
  PageMeta? meta(String id) {
    final m = _meta[id];
    if (m != null && (!_metaStale.contains(id) || _contentPending.contains(id))) return m;
    if (!_inactive && _metaLoading.add(id)) {
      _metaStale.remove(id);
      final version = _metaVersions[id] ?? 0;
      vault.loadPage(id).then((p) {
        _metaLoading.remove(id);
        if (_inactive) return;
        if (version != (_metaVersions[id] ?? 0)) {
          if (!_meta.containsKey(id) || _metaStale.contains(id)) meta(id);
          return;
        }
        _meta[id] = PageMeta.of(p);
        notifyListeners();
      }).catchError((Object _) {
        _metaLoading.remove(id);
        if (_inactive) return;
        if (version != (_metaVersions[id] ?? 0)) {
          if (!_meta.containsKey(id) || _metaStale.contains(id)) meta(id);
          return;
        }
        _meta[id] = PageMeta(id, '', null, null, null, true, missing: true);
        notifyListeners();
      });
    }
    return null;
  }

  /// The page's ancestors, top first, as far as they are loaded.
  List<PageMeta> ancestors(String id) {
    final out = <PageMeta>[];
    var p = meta(id)?.parent;
    for (var i = 0; p != null && i < 32; i++) {
      final m = meta(p);
      if (m == null) break;
      out.insert(0, m);
      p = m.parent;
    }
    return out;
  }

  /// Caches what a loaded page says about itself (the editor calls this
  /// after loading, so the page's options show without a second load).
  void adopt(PageData p) {
    final had = _meta[p.id];
    _metaVersions[p.id] = (_metaVersions[p.id] ?? 0) + 1;
    _meta[p.id] = PageMeta.of(p);
    _metaStale.remove(p.id);
    final now = _meta[p.id]!;
    if (had == null ||
        had.prefs != p.prefs ||
        had.title != p.title ||
        had.color != p.color ||
        had.firstLine != now.firstLine) {
      notifyListeners();
    }
  }

  // ---- page display options (stored in the page, so they sync) ----

  /// Options being written, by page (shown until the write completes).
  final Map<String, PagePrefs> _prefsPending = {};
  final Map<String, int> _prefsWrites = {};

  /// Full width, small text, lock and cover position of page [id], from the
  /// page (the defaults while it loads).
  PagePrefs prefs(String id) {
    final p = _prefsPending[id];
    if (p != null) return p.copy();
    // A stale entry still answers while it reloads.
    final m = meta(id) ?? _meta[id];
    return m == null ? PagePrefs() : PagePrefs.of(m.prefs);
  }

  /// Changes page [id]'s options and writes them to the core.
  Future<void> updatePrefs(String id, void Function(PagePrefs p) change) async {
    final next = prefs(id);
    change(next);
    _prefsPending[id] = next;
    final n = _prefsWrites[id] = (_prefsWrites[id] ?? 0) + 1;
    notifyListeners();
    try {
      await vault.run((v) => v.setPagePrefs(id: id, prefs: next.toData()), writes: true);
      final m = _meta[id];
      if (m != null) _meta[id] = m.withPrefs(next.toData());
    } catch (e) {
      debugPrint('set_page_prefs failed: $e');
    } finally {
      if (_prefsWrites[id] == n && !_inactive) {
        _prefsPending.remove(id);
        notifyListeners();
      }
    }
  }

  void _onChanges(ChangeBatch b) {
    if (_inactive) return;
    _changeVersion++;
    _generation++;
    _loading.clear();
    _docScanning = false;
    _changed.addAll(b.changedPages);
    _needsReload = _needsReload || b.treeChanged ||
        b.changes.any((c) => c.page == null);
    if (_needsReload) {
      _contentPending.clear();
    } else {
      _contentPending.addAll(b.changedPages);
    }
    final stale = b.treeChanged
        ? {..._meta.keys, ..._metaLoading}
        : b.changedPages.where((id) => _meta.containsKey(id) || _metaLoading.contains(id)).toSet();
    _metaStale.addAll(stale);
    for (final id in stale) {
      _metaVersions[id] = (_metaVersions[id] ?? 0) + 1;
    }
    _reload.run(reloadDelay, _refreshChanges);
  }

  Future<void> _refreshChanges() async {
    if (_inactive) return;
    if (_refreshBusy) {
      _refreshAgain = true;
      return;
    }
    _refreshBusy = true;
    try {
      await _applyChanges();
    } finally {
      _refreshBusy = false;
      if (_refreshAgain && !_inactive) {
        _refreshAgain = false;
        _reload.run(Duration.zero, _refreshChanges);
      }
    }
  }

  Future<void> _applyChanges() async {
    final version = _changeVersion;
    var reloading = false;
    Future<void> fullReload() async {
      reloading = true;
      _contentPending.clear();
      await vault.flush();
      if (!_inactive && version == _changeVersion) await _reloadOnce();
    }
    final ids = Set<String>.of(_changed);
    final full = _needsReload;
    _changed.clear();
    _needsReload = false;
    if (full || ids.isEmpty) {
      await fullReload();
      if (!_inactive && version != _changeVersion) _needsReload = true;
      return;
    }
    final gen = ++_generation;
    _loading.clear();
    _docScanning = false;
    bool current() => !_inactive && gen == _generation && version == _changeVersion;
    try {
      final positions = <String, int>{
        for (final e in _listed.entries)
          if (ids.contains(e.value.id)) e.value.id: e.key,
      };
      // An unlisted page's old position/type cannot be inferred from the feed.
      if (positions.length != ids.length) {
        await fullReload();
        return;
      }
      // The core supplies exact row metadata and its tie-breaking order. Local
      // edits lead this prefix. Older sync stamps may not: reload in that case.
      // Content writes reach the list index on the background writer. Wait for
      // its acknowledgement rather than racing the feed (especially sync/tests).
      await vault.flush();
      if (!current()) return;
      final front = await vault.listPages(offset: 0, limit: ids.length);
      if (!current()) return;
      if (front.length != ids.length || !front.every((r) => ids.contains(r.id)) ||
          front.any((r) {
            final before = _listed[positions[r.id]]!;
            return before.parent != r.parent || before.isDatabase != r.isDatabase;
          })) {
        await fullReload();
        return;
      }
      final oldPositions = positions.values.toList()..sort();
      final shifted = <int, PageRow>{};
      for (final e in _listed.entries) {
        if (ids.contains(e.value.id)) continue;
        final removedBefore = oldPositions.where((p) => p < e.key).length;
        shifted[e.key + front.length - removedBefore] = e.value;
      }
      for (var i = 0; i < front.length; i++) {
        shifted[i] = front[i];
      }
      // A sparse chunk can need the preceding chunk's last row after shifting.
      // Fill only these boundary holes; preserve every other cached row.
      final chunks = <int, List<PageRow>>{};
      for (final c in _chunks.keys) {
        final rows = <PageRow>[];
        for (var i = c * chunk; i < (c + 1) * chunk && i < count; i++) {
          var row = shifted[i];
          if (row == null) {
            final boundary = await vault.listPages(offset: i, limit: 1);
            if (!current()) return;
            if (boundary.length != 1) {
              await fullReload();
              return;
            }
            row = shifted[i] = boundary.single;
          }
          rows.add(row);
        }
        chunks[c] = rows;
      }
      final metadata = <String, PageMeta>{};
      for (final id in ids.where(_meta.containsKey)) {
        metadata[id] = PageMeta.of(await vault.loadPage(id));
        if (!current()) return;
      }
      final docs = <PageRow>[];
      for (final r in front) {
        if (r.parent != null && !_isDatabase.containsKey(r.parent)) {
          _isDatabase[r.parent!] = (await vault.loadPage(r.parent!)).isDatabase;
          if (!current()) return;
        }
        if (r.parent == null || _isDatabase[r.parent] != true) docs.add(r);
      }
      // Reads can span a write whose batch has not reached Dart yet. Deliver
      // those changes before publishing, so a mixed index snapshot is retried.
      await feed.poll();
      if (!current()) return;
      _chunks
        ..clear()
        ..addAll(chunks);
      _listed
        ..clear()
        ..addAll(shifted);
      _docs.removeWhere((r) => ids.contains(r.id));
      _docs.insertAll(0, docs);
      _docScanned += oldPositions.where((p) => p >= _docScanned).length;
      for (final r in front) {
        _isDatabase[r.id] = r.isDatabase;
      }
      _meta.addAll(metadata);
      _metaStale.removeAll(metadata.keys);
      _contentPending.removeAll(ids);
      notifyListeners();
      if (current() && !_docDone && _docs.length < _docWanted) {
        await _scanDocs(gen);
      }
    } catch (e) {
      if (current()) {
        debugPrint('incremental pages refresh failed: $e');
        await fullReload();
      }
    } finally {
      // A newer batch superseded the reads, so it must include these edits too.
      if (!_inactive && version != _changeVersion) {
        _changed.addAll(ids);
        if (reloading) _needsReload = true;
      }
    }
  }

  void _onVaultClosed() {
    _reload.cancel();
    _generation++;
  }

  @override
  void dispose() {
    vault.removeCloseListener(_onVaultClosed);
    _disposed = true;
    _reload.cancel();
    feed.removeListener(_onChanges);
    super.dispose();
  }
}

/// The first non-empty line of [texts] (a page's blocks, in order), at
/// most 80 characters; empty when there is none.
String firstLineOfTexts(Iterable<String> texts) {
  for (final text in texts) {
    final t = text.trim();
    if (t.isEmpty) continue;
    final line = t.split('\n').first.trim();
    return line.length > 80 ? '${line.substring(0, 80)}…' : line;
  }
  return '';
}

/// A page's title, icon, cover, place and display options (see
/// [PagesStore.meta]).
class PageMeta {
  const PageMeta(this.id, this.title, this.icon, this.cover, this.parent, this.inTrash,
      {this.missing = false,
      this.pageType = documentType,
      this.color,
      this.firstLine = '',
      this.prefs = const PagePrefsData(smallText: false, fullWidth: false, locked: false)});

  PageMeta.of(PageData p)
      : this(p.id, p.title, p.icon, p.cover, p.parent, p.inTrash,
            prefs: p.prefs,
            pageType: p.pageType,
            color: p.color,
            firstLine: p.title.isEmpty ? firstLineOfTexts(p.blocks.map((b) => b.text)) : '');

  final String id, title;
  final String? icon, cover, parent, color;
  final bool inTrash;
  final PagePrefsData prefs;

  /// The first line of body text of an untitled page (empty for a titled
  /// one), which lists show in its place.
  final String firstLine;

  PageMeta withPrefs(PagePrefsData p) => PageMeta(id, title, icon, cover, parent, inTrash,
      missing: missing, pageType: pageType, prefs: p, color: color, firstLine: firstLine);

  /// The page does not exist (purged, or never synced here).
  final bool missing;

  /// The core's page type: `document`, `database`, or a type a mod defines
  /// (kept as it is).
  final String pageType;

  /// The page is a database (its icon defaults to the database glyph).
  bool get isDatabase => pageType == databaseType;
}

/// A page's display options (see [PagesStore.prefs]); the editable form of
/// the core's [PagePrefsData].
class PagePrefs {
  PagePrefs();

  PagePrefs.of(PagePrefsData d)
      : fullWidth = d.fullWidth,
        smallText = d.smallText,
        locked = d.locked,
        coverY = d.coverPosition == null ? 0.5 : d.coverPosition! / 1000;

  bool fullWidth = false;
  bool smallText = false;
  bool locked = false;

  /// Vertical position of the cover image, 0 (top) to 1 (bottom).
  double coverY = 0.5;

  PagePrefs copy() => PagePrefs()
    ..fullWidth = fullWidth
    ..smallText = smallText
    ..locked = locked
    ..coverY = coverY;

  PagePrefsData toData() {
    final pos = (coverY.clamp(0.0, 1.0) * 1000).round();
    return PagePrefsData(
      smallText: smallText,
      fullWidth: fullWidth,
      locked: locked,
      coverPosition: pos == 500 ? null : pos,
    );
  }
}
