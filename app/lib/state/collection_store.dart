// One database (a collection page): its schema, views and default template,
// and the rows of the active view, loaded by window.
//
// - Rows come in windows of [RowSource.window] from `query` with the view's
//   filter (plus the search) and sorts; views ask for rows by index and only
//   the windows they draw are loaded.
// - A grouped view (board) has one [RowSource] per group, with the group's
//   condition added to the filter.
// - Calendar and timeline ask for a date range ([showRange]); only entries
//   overlapping it are loaded.
// - Edits patch the loaded rows at once (optimistic) and then write to the
//   core. Changes from anywhere arrive through the change feed; the store
//   reloads the windows it has, keeping the old rows on screen until the new
//   ones arrive.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';

import '../core/change_feed.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/vault.dart';
import '../ui/database/config.dart';
import '../ui/database/values.dart';

/// Rows of one query, loaded by window.
class RowSource {
  RowSource(this._store, this.filter, this.sorts);

  final CollectionStore _store;
  Filter? filter;
  List<SortSpec> sorts;

  static const window = 100;

  /// Windows kept in memory; those farthest from the latest load go first.
  static const maxWindows = 8;

  final Map<int, List<PageRow>> _windows = {};
  final Set<int> _loading = {};

  /// Number of rows (-1 until the first window has loaded).
  int total = -1;

  bool get loaded => total >= 0;

  /// Row [i], or null while its window loads (the load starts here).
  PageRow? rowAt(int i) {
    final w = (i ~/ window) * window;
    final rows = _windows[w];
    if (rows == null) {
      _load(w);
      return null;
    }
    return i - w < rows.length ? rows[i - w] : null;
  }

  /// The loaded rows in order (gaps skipped).
  Iterable<PageRow> get loadedRows sync* {
    final keys = _windows.keys.toList()..sort();
    for (final k in keys) {
      yield* _windows[k]!;
    }
  }

  /// Index of row [id] among the loaded rows, or -1.
  int indexOf(String id) {
    for (final e in _windows.entries) {
      final i = e.value.indexWhere((r) => r.id == id);
      if (i >= 0) return e.key + i;
    }
    return -1;
  }

  Future<void> _load(int offset) async {
    if (_store._inactive) return;
    if (_loading.contains(offset)) return;
    _loading.add(offset);
    final gen = _store._gen;
    try {
      final r = await _store.vault.run((v) => v.query(
          scope: _store.collection, filter: filter, sorts: sorts, offset: offset, limit: window));
      if (_store._inactive || gen != _store._gen) return;
      total = r.total;
      _windows[offset] = r.rows;
      _evict(offset);
      _store._changed();
    } catch (e) {
      debugPrint('query failed: $e');
    } finally {
      _loading.remove(offset);
    }
  }

  void _evict(int near) {
    if (_windows.length <= maxWindows) return;
    final far = _windows.keys.toList()..sort((a, b) => (b - near).abs().compareTo((a - near).abs()));
    for (final k in far.take(_windows.length - maxWindows)) {
      _windows.remove(k);
    }
  }

  /// Loads the first window if nothing is loaded yet.
  Future<void> ensure() => loaded || _loading.isNotEmpty ? Future.value() : _load(0);

  /// Reloads the windows it has (or the first), replacing them when all
  /// have arrived.
  Future<void> reload() async {
    if (_store._inactive) return;
    final offsets = _windows.keys.isEmpty ? [0] : (_windows.keys.toList()..sort());
    final gen = _store._gen;
    final results = await Future.wait(offsets.map((o) => _store.vault.run(
        (v) => v.query(scope: _store.collection, filter: filter, sorts: sorts, offset: o, limit: window))));
    if (_store._inactive || gen != _store._gen) return;
    _windows.clear();
    for (var i = 0; i < offsets.length; i++) {
      _windows[offsets[i]] = results[i].rows;
      total = results[i].total;
    }
    // Windows past the end are dropped.
    _windows.removeWhere((o, rows) => o > 0 && o >= total);
  }

  void reset(Filter? f, List<SortSpec> s) {
    filter = f;
    sorts = s;
    _windows.clear();
    total = -1;
  }

  bool patch(String id, PageRow Function(PageRow r) f) {
    var hit = false;
    for (final rows in _windows.values) {
      for (var i = 0; i < rows.length; i++) {
        if (rows[i].id == id) {
          rows[i] = f(rows[i]);
          hit = true;
        }
      }
    }
    return hit;
  }

  bool remove(String id) {
    var hit = false;
    for (final e in _windows.entries) {
      final n = e.value.length;
      e.value.removeWhere((r) => r.id == id);
      if (e.value.length != n) hit = true;
    }
    if (hit) total--;
    return hit;
  }

  /// Moves row [id] after [after] (null: first) among the loaded rows.
  void moveLocal(String id, String? after) {
    final all = loadedRows.toList();
    final i = all.indexWhere((r) => r.id == id);
    if (i < 0) return;
    final row = all.removeAt(i);
    final j = after == null ? 0 : all.indexWhere((r) => r.id == after) + 1;
    all.insert(j.clamp(0, all.length), row);
    final keys = _windows.keys.toList()..sort();
    var k = 0;
    for (final o in keys) {
      final n = _windows[o]!.length;
      _windows[o] = all.sublist(k, (k + n).clamp(0, all.length));
      k += n;
    }
  }

  /// Adds [row] at the end of the loaded rows (a new row).
  void append(PageRow row) {
    total = total < 0 ? 1 : total + 1;
    if (_windows.isEmpty) {
      _windows[0] = [row];
      return;
    }
    final last = (_windows.keys.toList()..sort()).last;
    if (_windows[last]!.length < window) _windows[last]!.add(row);
  }
}

/// One board column: the group, its key and its rows.
class GroupSource {
  GroupSource(this.group, this.key, this.rows);
  GroupRow group;
  final String key;
  final RowSource rows;
}

/// Views kept outside the collection's saved views: the view blocks of a
/// page. The store reads [views] instead of `list_views` and writes view
/// changes here; listeners hear when the views changed elsewhere (an edit
/// of the blocks, undo, sync).
abstract class ViewSource extends ChangeNotifier {
  List<ViewDef> get views;

  /// Saves [v] (same id).
  Future<void> save(ViewDef v);

  /// Adds [v] after view [after] (null: after the last) and returns its id.
  Future<String> add(ViewDef v, {String? after});

  Future<void> delete(ViewDef v);
}

class CollectionStore extends ChangeNotifier {
  /// [withRows] false: only the schema, views and template (page
  /// property rows). [viewSource]: the views come from there (view
  /// blocks), not from the collection's saved views.
  CollectionStore(this.vault, this.feed, this.collection, {this.withRows = true, this.viewSource}) {
    feed.addListener(_onChanges);
    vault.addCloseListener(_onVaultClosed);
    main = RowSource(this, null, const []);
    final src = viewSource;
    if (src != null) {
      views = src.views;
      src.addListener(_onSourceViews);
    }
  }

  /// Where the views live when they are not the saved views (null).
  final ViewSource? viewSource;

  final Vault vault;
  final ChangeFeed feed;

  /// The collection page's id.
  final String collection;
  final bool withRows;

  String title = '';
  String? icon;
  String? resolvedSpaceColor;
  final Set<String> _spaceAncestors = {};
  List<PropDef> schema = const [];
  List<ViewDef> views = const [];
  String? templateId;
  bool loaded = false;

  /// The core's page type (see [documentType] and [databaseType]).
  String pageType = documentType;

  /// The page is a database (false: a plain page; [schema] is empty).
  bool get isDatabase => pageType == databaseType;

  /// The view shown (null: the first one).
  String? activeView;

  /// Title search (the toolbar's search box).
  String search = '';

  late final RowSource main;
  List<GroupSource> groups = const [];

  /// Date range of the calendar or timeline, and its entries.
  String? rangeProp;
  int rangeFrom = 0, rangeTo = 0;
  List<RangeEntry> rangeEntries = const [];
  bool rangeLoaded = false;

  /// Footer calculations: "key|calc" -> text.
  final Map<String, String> calcResults = {};
  final Set<String> _calcLoading = {};

  int _gen = 0;
  int _writes = 0;
  final _reload = Debounce();
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;
  bool _schemaDirty = false;

  ViewDef? get view {
    if (views.isEmpty) return null;
    return views.firstWhere((v) => v.id == activeView, orElse: () => views.first);
  }

  PropDef? def(String key) {
    for (final d in schema) {
      if (d.key == key) return d;
    }
    return null;
  }

  /// The properties [v] shows after the title, in its order.
  List<PropDef> visibleProps([ViewDef? v]) {
    v ??= view;
    if (v == null) return const [];
    return [for (final k in v.visibleProps) ?def(k)];
  }

  List<PropDef> hiddenProps([ViewDef? v]) {
    v ??= view;
    final shown = v?.visibleProps.toSet() ?? const {};
    return [for (final d in schema) if (!shown.contains(d.key)) d];
  }

  PropDef? get groupDef => view?.groupBy == null ? null : def(view!.groupBy!);

  Filter? get _searchFilter =>
      search.trim().isEmpty ? null : Filters.cond(BuiltinKeys.title, FilterOp.contains, Values.text(search.trim()));

  /// The active view's filter with the search.
  Filter? get filter => Filters.both(Filters.prune(view?.filter), _searchFilter);

  bool _notifyQueued = false;

  /// Notifies listeners; a change made while the frame is being built (an
  /// editor saving from dispose, say) is announced after that frame.
  void _changed() {
    if (_inactive) return;
    if (SchedulerBinding.instance.schedulerPhase != SchedulerPhase.persistentCallbacks) {
      notifyListeners();
      return;
    }
    if (_notifyQueued) return;
    _notifyQueued = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _notifyQueued = false;
      if (!_inactive) notifyListeners();
    });
  }

  // ------------------------------------------------------------- loading

  Future<void> load() async {
    if (_inactive) return;
    await _loadMeta();
    if (withRows) await _loadRows();
    _changed();
  }

  Future<void> _loadMeta() async {
    if (_inactive) return;
    final src = viewSource;
    final r = await Future.wait([
      vault.run((v) => v.getSchema(collection: collection)),
      src != null ? Future.value(src.views) : vault.run((v) => v.listViews(collection: collection)),
      vault.run((v) => v.defaultTemplate(collection: collection)),
      vault.run((v) => v.loadPage(id: collection)),
    ]);
    if (_inactive) return;
    schema = r[0] as List<PropDef>;
    views = src != null ? src.views : r[1] as List<ViewDef>;
    templateId = r[2] as String?;
    final p = r[3] as PageData;
    final ancestors = <String>{collection};
    var ancestor = p;
    while (ancestor.parent != null && ancestors.add(ancestor.parent!)) {
      ancestor = await vault.run((v) => v.loadPage(id: ancestor.parent!));
      if (_inactive) return;
    }
    resolvedSpaceColor = ancestor.color;
    _spaceAncestors..clear()..addAll(ancestors);
    title = p.title;
    icon = p.icon;
    pageType = p.pageType;
    loaded = true;
  }

  /// Rebuilds the sources for the active view and loads their first rows.
  Future<void> _loadRows() async {
    if (_inactive) return;
    _gen++;
    calcResults.clear();
    final v = view;
    main.reset(filter, v?.sorts ?? const []);
    groups = const [];
    rangeLoaded = false;
    _changed();
    final futures = <Future<void>>[main.ensure()];
    if (v != null && v.groupBy != null && groupDef != null) futures.add(_loadGroups(rebuild: true));
    if (rangeProp != null) futures.add(_loadRange());
    await Future.wait(futures);
    _changed();
  }

  /// A loaded page as a row.
  static PageRow rowOf(PageData p) => PageRow(
        id: p.id,
        title: p.title,
        parent: p.parent,
        updatedMs: p.updatedMs,
        createdMs: p.createdMs,
        childCount: 0,
        icon: p.icon,
        pageType: p.pageType,
        favorite: p.favorite,
        properties: p.properties,
      );

  static String groupKey(PropValue v) => switch (v.kind) {
        ValueKind.empty => '',
        ValueKind.checkbox => v.checked ? 'true' : 'false',
        ValueKind.select || ValueKind.status => v.text,
        ValueKind.multiSelect || ValueKind.relation => v.ids.join(','),
        ValueKind.number => '${v.number}',
        ValueKind.date => '${v.startMs}',
        _ => v.text,
      };

  /// The condition that selects group [g] of [key].
  static Filter groupFilter(String key, PropValue g) {
    if (g.kind == ValueKind.empty) return Filters.cond(key, FilterOp.isEmpty);
    if (g.kind == ValueKind.checkbox) return Filters.cond(key, g.checked ? FilterOp.checked : FilterOp.unchecked);
    // A multi-select row belongs to each of its options' groups.
    if (g.kind == ValueKind.multiSelect) return Filters.cond(key, FilterOp.contains, g);
    return Filters.cond(key, FilterOp.is_, g);
  }

  Future<void> _loadGroups({bool rebuild = false}) async {
    if (_inactive) return;
    final v = view;
    final gd = groupDef;
    if (v == null || gd == null) {
      groups = const [];
      return;
    }
    final gen = _gen;
    final f = filter;
    final rows = await vault.run((r) => r.queryGroups(scope: collection, filter: f, groupBy: gd.key));
    if (_inactive || gen != _gen) return;
    final old = {for (final g in groups) g.key: g};
    final next = <GroupSource>[];
    for (final g in rows) {
      final k = groupKey(g.value);
      final prev = rebuild ? null : old[k];
      if (prev != null) {
        prev.group = g;
        next.add(prev);
      } else {
        next.add(GroupSource(g, k, RowSource(this, Filters.both(f, groupFilter(gd.key, g.value)), v.sorts)));
      }
    }
    groups = next;
    await Future.wait([for (final g in next) if (!g.rows.loaded) g.rows.ensure()]);
  }

  /// Shows entries of [prop] overlapping [from, to) (calendar, timeline).
  void showRange(String prop, int from, int to) {
    if (prop == rangeProp && from == rangeFrom && to == rangeTo) return;
    rangeProp = prop;
    rangeFrom = from;
    rangeTo = to;
    _loadRange();
  }

  Future<void> _loadRange() async {
    if (_inactive) return;
    final p = rangeProp;
    if (p == null) return;
    final gen = _gen;
    final (from, to) = (rangeFrom, rangeTo);
    try {
      final r = await vault.run(
          (v) => v.queryRange(scope: collection, dateProp: p, fromMs: from, toMs: to, filter: filter));
      if (_inactive || gen != _gen || from != rangeFrom || to != rangeTo || p != rangeProp) return;
      rangeEntries = r;
      rangeLoaded = true;
      _changed();
    } catch (e) {
      debugPrint('range failed: $e');
    }
  }

  /// Reloads everything loaded, keeping what is shown until it arrives.
  Future<void> refresh() async {
    if (_inactive) return;
    final writes = _writes;
    if (_schemaDirty) {
      _schemaDirty = false;
      await _loadMeta();
      if (_inactive) return;
    }
    if (!withRows) {
      _changed();
      return;
    }
    final gen = _gen;
    await Future.wait([
      main.reload(),
      if (groupDef != null) _loadGroups().then((_) => Future.wait([for (final g in groups) g.rows.reload()])),
      if (rangeProp != null) _loadRange(),
    ]);
    if (_inactive || gen != _gen) return;
    calcResults.clear();
    _changed();
    // A write started meanwhile: its echo reloads again.
    if (writes != _writes) _schedule();
  }

  void _onChanges(ChangeBatch b) {
    var relevant = b.reset || b.treeChanged;
    if (b.reset || b.treeChanged || _spaceAncestors.any(b.touches) || b.touches(collection)) {
      relevant = true;
      _schemaDirty = true;
    }
    if (!relevant && withRows) {
      final ids = b.changedPages.toSet();
      relevant = main.loadedRows.any((r) => ids.contains(r.id)) ||
          groups.any((g) => g.rows.loadedRows.any((r) => ids.contains(r.id))) ||
          rangeEntries.any((e) => ids.contains(e.page.id));
    }
    if (relevant) _schedule();
  }

  void _schedule() {
    if (_inactive) return;
    _reload.run(const Duration(milliseconds: 150), () {
      if (!_inactive) refresh();
    });
  }

  // ---------------------------------------------------------------- views

  /// The view blocks changed: take their views, reloading rows when the
  /// shown view's query changed.
  void _onSourceViews() {
    if (_inactive) return;
    final old = view;
    views = viewSource!.views;
    final now = view;
    if (old == null || now == null || old.id != now.id || _requery(old, now)) {
      if (old?.kind != now?.kind || old?.dateProp != now?.dateProp || old?.id != now?.id) rangeProp = null;
      if (loaded) _loadRows();
    } else {
      _changed();
    }
  }

  static bool _requery(ViewDef old, ViewDef v) =>
      old.filter != v.filter ||
      !listEquals(old.sorts, v.sorts) ||
      old.groupBy != v.groupBy ||
      old.kind != v.kind ||
      old.dateProp != v.dateProp;

  void selectView(String id) {
    if (activeView == id) return;
    activeView = id;
    rangeProp = null;
    rangeEntries = const [];
    _loadRows();
  }

  void setSearch(String q) {
    if (q == search) return;
    search = q;
    _loadRows();
  }

  /// Replaces view [v] (same id) and saves it; reloads rows when what they
  /// depend on changed.
  Future<void> updateView(ViewDef v) async {
    final old = views.firstWhere((x) => x.id == v.id, orElse: () => v);
    views = [for (final x in views) x.id == v.id ? v : x];
    if (_requery(old, v) && v.id == view?.id) {
      if (old.kind != v.kind || old.dateProp != v.dateProp) rangeProp = null;
      _loadRows();
    } else {
      _changed();
    }
    final src = viewSource;
    if (src != null) return src.save(v);
    await _write((r) => r.updateView(collection: collection, view: v));
  }

  Future<void> updateConfig(ViewDef v, void Function(ViewConfig c) f) {
    final c = v.cfg;
    f(c);
    return updateView(v.copyWith(config: c.encode()));
  }

  /// Adds a view of [kind] and shows it.
  Future<String> addView(ViewKind kind, {String? name}) async {
    final cur = view;
    final dateProp = kind == ViewKind.calendar || kind == ViewKind.timeline ? firstDateKey() : null;
    final group = kind == ViewKind.board ? firstGroupKey() : null;
    final v = ViewDef(
      id: '',
      name: name ?? viewKindLabel(kind),
      kind: kind,
      filter: null,
      sorts: const [],
      groupBy: group,
      dateProp: dateProp,
      visibleProps: cur?.visibleProps ?? [for (final d in schema) d.key],
      cardCover: null,
      config: '',
    );
    final src = viewSource;
    final id = src != null ? await src.add(v) : await _write((r) => r.addView(collection: collection, view: v));
    views = [...views.where((x) => x.id != id), v.copyWith(id: id)];
    selectView(id);
    return id;
  }

  Future<void> duplicateView(ViewDef v) async {
    final copy = v.copyWith(id: '', name: '${v.name} (copy)');
    final src = viewSource;
    if (src != null) {
      final id = await src.add(copy, after: v.id);
      final i = views.indexWhere((x) => x.id == v.id);
      views = [...views.where((x) => x.id != id)]..insert(i + 1, copy.copyWith(id: id));
      selectView(id);
      return;
    }
    final id = await _write((r) => r.addView(collection: collection, view: copy));
    final i = views.indexWhere((x) => x.id == v.id);
    views = [...views, copy.copyWith(id: id)];
    await _write((r) => r.moveView(collection: collection, viewId: id, index: i + 1));
    views = [...views.where((x) => x.id != id)]..insert(i + 1, copy.copyWith(id: id));
    selectView(id);
  }

  /// Whether view [v] can be deleted: saved views keep at least one; a view
  /// block can always go (it is a block of the page).
  bool canDeleteView(ViewDef v) => viewSource != null || views.length > 1;

  Future<void> deleteView(ViewDef v) async {
    final src = viewSource;
    if (src != null) {
      final rest = [for (final x in views) if (x.id != v.id) x];
      if (rest.isNotEmpty && (activeView == v.id || view?.id == v.id)) {
        final i = views.indexWhere((x) => x.id == v.id);
        activeView = rest[(i - 1).clamp(0, rest.length - 1)].id;
      }
      return src.delete(v);
    }
    if (views.length <= 1) return;
    final i = views.indexWhere((x) => x.id == v.id);
    views = [for (final x in views) if (x.id != v.id) x];
    if (activeView == v.id || (activeView == null && i == 0)) {
      activeView = views[(i - 1).clamp(0, views.length - 1)].id;
      rangeProp = null;
      _loadRows();
    } else {
      _changed();
    }
    await _write((r) => r.deleteView(collection: collection, viewId: v.id));
  }

  Future<void> moveView(String id, int index) async {
    final v = views.firstWhere((x) => x.id == id);
    final rest = [for (final x in views) if (x.id != id) x];
    rest.insert(index.clamp(0, rest.length), v);
    views = rest;
    _changed();
    if (viewSource != null) return;
    await _write((r) => r.moveView(collection: collection, viewId: id, index: index));
  }

  String? firstDateKey() {
    for (final d in schema) {
      if (d.kind == PropKind.date) return d.key;
    }
    return null;
  }

  String? firstGroupKey() {
    for (final k in [PropKind.status, PropKind.select, PropKind.multiSelect, PropKind.checkbox]) {
      for (final d in schema) {
        if (d.kind == k) return d.key;
      }
    }
    return null;
  }

  // ---------------------------------------------------------------- rows

  Iterable<RowSource> get _sources sync* {
    yield main;
    for (final g in groups) {
      yield g.rows;
    }
  }

  void _patch(String id, PageRow Function(PageRow r) f) {
    for (final s in _sources) {
      s.patch(id, f);
    }
    rangeEntries = [
      for (final e in rangeEntries)
        e.page.id == id
            ? RangeEntry(page: f(e.page), prop: e.prop, startMs: e.startMs, endMs: e.endMs, hasTime: e.hasTime)
            : e,
    ];
    _changed();
  }

  Future<T> _write<T>(Future<T> Function(NotesVault v) f) {
    _writes++;
    return vault.run(f, writes: true);
  }

  /// Sets property [key] of row [id] (title: [BuiltinKeys.title] with a
  /// text value).
  Future<void> setValue(String id, String key, PropValue v) async {
    if (key == BuiltinKeys.title) return renameRow(id, v.text);
    _patch(id, (r) => r.withValue(key, v));
    // A date moved in a calendar or timeline: move its entry too.
    if (v.kind == ValueKind.date && key == rangeProp) {
      rangeEntries = [
        for (final e in rangeEntries)
          e.page.id == id && e.prop == key
              ? RangeEntry(page: e.page, prop: e.prop, startMs: v.startMs, endMs: v.endMs, hasTime: v.hasTime)
              : e,
      ];
    }
    if (v.isEmpty) {
      await _write((r) => r.removeProperty(page: id, key: key));
    } else {
      await _write((r) => r.setProperty(page: id, key: key, value: v));
    }
  }

  Future<void> renameRow(String id, String t) async {
    _patch(id, (r) => r.copyWith(title: t));
    await _write((r) => r.renamePage(id: id, title: t));
  }

  Future<void> setRowIcon(String id, String? icon) async {
    _patch(id, (r) => r.copyWith(icon: icon));
    await _write((r) => r.setIcon(id: id, icon: icon));
  }

  /// Creates a row (from the default template unless [empty]) with
  /// [values] and returns its id. It shows at the end of the loaded rows
  /// until the reload.
  Future<String> addRow(
      {String title = '', Map<String, PropValue> values = const {}, String? groupKey, bool empty = false}) async {
    final id = await _write((r) => r.createPage(parent: collection, title: title));
    if (empty && templateId != null) await _clearRow(id);
    for (final e in values.entries) {
      if (e.value.isEmpty) continue;
      await _write((r) => r.setProperty(page: id, key: e.key, value: e.value));
    }
    final made = rowOf(await vault.run((r) => r.loadPage(id: id)));
    if (main.indexOf(id) < 0) main.append(made);
    if (groupKey != null) {
      for (final g in groups) {
        if (g.key == groupKey && g.rows.indexOf(id) < 0) g.rows.append(made);
      }
    }
    _changed();
    return id;
  }

  /// Removes what the default template copied into new row [id]: its
  /// blocks, property values and icon. (The core always copies the
  /// template; an empty page is a copy undone.)
  Future<void> _clearRow(String id) async {
    final p = await vault.run((r) => r.loadPage(id: id));
    if (p.blocks.isNotEmpty) {
      await _write((r) => r.apply(pageId: id, ops: [
            for (final b in p.blocks.reversed)
              BOp(op: OpKind.delete, id: b.id, kind: b.kind, text: '', marks: const [], checked: false),
          ]));
    }
    for (final pr in p.properties) {
      await _write((r) => r.removeProperty(page: id, key: pr.key));
    }
    if (p.icon != null) await _write((r) => r.setIcon(id: id, icon: null));
  }

  Future<void> deleteRow(String id) async {
    for (final s in _sources) {
      s.remove(id);
    }
    rangeEntries = [for (final e in rangeEntries) if (e.page.id != id) e];
    _changed();
    await _write((r) => r.deletePage(id: id));
  }

  Future<String> duplicateRow(String id) => _write((r) => r.duplicatePage(id: id));

  /// Moves row [id] after row [after] (null: first) in the collection's
  /// order (board cards without a sort).
  Future<void> moveRow(String id, String? after, {RowSource? within}) async {
    within?.moveLocal(id, after);
    _changed();
    await _write((r) => r.movePage(id: id, newParent: collection, after: after));
  }

  /// Moves a board card from one group to another, placing it after
  /// [after] (null: first).
  Future<void> moveToGroup(String id, GroupSource from, GroupSource to, String? after) async {
    final gd = groupDef;
    if (gd == null) return;
    final row = from.rows.loadedRows.firstWhere((r) => r.id == id);
    PropValue next;
    if (gd.kind == PropKind.multiSelect) {
      final ids = [...row.value(gd.key).ids]..remove(from.key);
      if (to.key.isNotEmpty && !ids.contains(to.key)) ids.add(to.key);
      next = Values.multi(ids);
    } else if (gd.kind == PropKind.checkbox) {
      next = Values.checkbox(to.key == 'true');
    } else {
      next = to.group.value;
    }
    final moved = row.withValue(gd.key, next);
    from.rows.remove(id);
    from.group = _count(from.group, -1);
    to.group = _count(to.group, 1);
    to.rows.total = to.rows.total < 0 ? 1 : to.rows.total + 1;
    final list = to.rows._windows[0] ?? (to.rows._windows[0] = []);
    final j = after == null ? 0 : list.indexWhere((r) => r.id == after) + 1;
    list.insert(j.clamp(0, list.length), moved);
    main.patch(id, (_) => moved);
    _changed();
    await setValue(id, gd.key, next);
    if (view?.sorts.isEmpty ?? true) {
      await _write((r) => r.movePage(id: id, newParent: collection, after: after));
    }
  }

  static GroupRow _count(GroupRow g, int d) => GroupRow(value: g.value, label: g.label, color: g.color, count: g.count + d);

  // --------------------------------------------------------------- schema

  /// Saves [next] as the schema; returns it as the core normalised it.
  Future<List<PropDef>> setSchema(List<PropDef> next) async {
    schema = next;
    _changed();
    final promotes = loaded && pageType == documentType;
    final r = await _write((v) => v.setSchema(collection: collection, schema: next));
    if (promotes) {
      // The core gives a document its first saved view when it gets
      // properties; the page shows its children through view blocks, so
      // the saved view goes again before it is migrated into a block.
      pageType = databaseType;
      for (final v in await vault.run((x) => x.listViews(collection: collection))) {
        await _write((x) => x.deleteView(collection: collection, viewId: v.id));
      }
    }
    if (_inactive) return r;
    schema = r;
    _changed();
    return r;
  }

  /// [name] (or the kind's default name), numbered when the schema has it.
  String freePropName(PropKind kind, {String? name}) {
    final base = name ?? defaultPropName(kind);
    var n = base;
    var i = 1;
    while (schema.any((d) => d.name == n)) {
      n = '$base ${i++}';
    }
    return n;
  }

  /// Adds a property of [kind] and shows it in [inView] (all table views
  /// when null) at [index] (the end when null). Returns its key.
  Future<String> addProperty(PropKind kind, {String? name, ViewDef? inView, int? index}) async {
    final n = freePropName(kind, name: name);
    final before = schema.map((d) => d.key).toSet();
    final r = await setSchema([
      ...schema,
      PropDef(key: '', name: n, kind: kind, options: defaultOptions(kind), config: defaultConfig(kind)),
    ]);
    final key = r.firstWhere((d) => !before.contains(d.key)).key;
    final targets = inView != null ? [views.firstWhere((v) => v.id == inView.id, orElse: () => inView)] : views;
    for (final v in targets) {
      final vis = [...v.visibleProps];
      vis.insert((index ?? vis.length).clamp(0, vis.length), key);
      await updateView(v.copyWith(visibleProps: vis));
    }
    return key;
  }

  /// Saves [d] (same key); returns it as saved (new options get ids).
  Future<PropDef> updateProperty(PropDef d) async {
    final r = await setSchema([for (final x in schema) x.key == d.key ? d : x]);
    return r.firstWhere((x) => x.key == d.key, orElse: () => d);
  }

  Future<PropDef> changeType(PropDef d, PropKind kind) {
    final opts = d.hasOptions ? d.options : const <SelectOption>[];
    final next = d.copyWith(
      kind: kind,
      options: kind == PropKind.status && opts.isEmpty ? defaultOptions(kind) : opts,
      config: kind == d.kind ? d.config : defaultConfig(kind),
    );
    return updateProperty(next);
  }

  Future<void> deleteProperty(String key) async {
    for (final v in views) {
      if (v.visibleProps.contains(key)) {
        views = [for (final x in views) x.id == v.id ? x.copyWith(visibleProps: [...x.visibleProps]..remove(key)) : x];
      }
    }
    await setSchema([for (final x in schema) if (x.key != key) x]);
    await load();
  }

  /// Copies property [d] (and its values) next to it.
  Future<String> duplicateProperty(PropDef d, {ViewDef? inView}) async {
    final v = inView ?? view;
    final idx = v == null ? null : v.visibleProps.indexOf(d.key) + 1;
    final key = await addProperty(d.kind, name: '${d.name} (1)', inView: v, index: idx);
    final copy = def(key)!;
    await updateProperty(copy.copyWith(options: d.options, config: d.config));
    if (!d.computed) {
      final all = await vault.run((r) => r.query(
          scope: collection,
          filter: Filters.cond(d.key, FilterOp.isNotEmpty),
          sorts: const [],
          offset: 0,
          limit: 1 << 30));
      for (final row in all.rows) {
        final val = row.value(d.key);
        await _write((r) => r.setProperty(page: row.id, key: key, value: val));
      }
    }
    return key;
  }

  static String defaultPropName(PropKind k) => switch (k) {
        PropKind.text => 'Text',
        PropKind.number => 'Number',
        PropKind.select => 'Select',
        PropKind.multiSelect => 'Multi-select',
        PropKind.status => 'Status',
        PropKind.date => 'Date',
        PropKind.checkbox => 'Checkbox',
        PropKind.url => 'URL',
        PropKind.email => 'Email',
        PropKind.phone => 'Phone',
        PropKind.relation => 'Relation',
        PropKind.createdTime => 'Created time',
        PropKind.updatedTime => 'Last edited time',
        PropKind.files => 'Files & media',
        PropKind.formula => 'Formula',
        PropKind.rollup => 'Rollup',
      };

  static List<SelectOption> defaultOptions(PropKind k) => k == PropKind.status
      ? const [
          SelectOption(id: '', name: 'Not started', color: 'default'),
          SelectOption(id: '', name: 'In progress', color: 'blue'),
          SelectOption(id: '', name: 'Done', color: 'green'),
        ]
      : const [];

  static String defaultConfig(PropKind k) => '';

  // ------------------------------------------------------------- templates

  Future<void> setTemplate(String? id) async {
    templateId = id;
    _changed();
    await _write((r) => r.setDefaultTemplate(collection: collection, template: id));
  }

  /// Creates an empty template and makes it the default.
  Future<String> newTemplate() async {
    final id = await _write((r) => r.createPage(parent: collection, title: ''));
    await setTemplate(id);
    return id;
  }

  // ---------------------------------------------------------- calculations

  /// The footer text of [calc] on [d] (null while it loads).
  String? calc(PropDef? d, String key, Calc calc) {
    final k = '$key|${calc.name}';
    final have = calcResults[k];
    if (have != null || _calcLoading.contains(k)) return have;
    _calcLoading.add(k);
    final gen = _gen;
    _computeCalc(d, key, calc).then((s) {
      _calcLoading.remove(k);
      if (_inactive || gen != _gen) return;
      calcResults[k] = s;
      _changed();
    }, onError: (Object e) {
      _calcLoading.remove(k);
      debugPrint('calc failed: $e');
    });
    return null;
  }

  Future<int> _count2(Filter? f) async =>
      (await vault.run((r) => r.query(scope: collection, filter: f, sorts: const [], offset: 0, limit: 1))).total;

  Future<String> _computeCalc(PropDef? d, String key, Calc c) async {
    final f = filter;
    String pct(int a, int b) => b == 0 ? '0%' : '${(a * 100 / b).toStringAsFixed(a * 100 % b == 0 ? 0 : 1)}%';
    final all = await _count2(f);
    switch (c) {
      case Calc.none:
        return '';
      case Calc.countAll:
        return '$all';
      case Calc.countValues || Calc.countNotEmpty:
        return '${await _count2(Filters.both(f, Filters.cond(key, FilterOp.isNotEmpty)))}';
      case Calc.countEmpty:
        return '${await _count2(Filters.both(f, Filters.cond(key, FilterOp.isEmpty)))}';
      case Calc.percentEmpty:
        return pct(await _count2(Filters.both(f, Filters.cond(key, FilterOp.isEmpty))), all);
      case Calc.percentNotEmpty:
        return pct(await _count2(Filters.both(f, Filters.cond(key, FilterOp.isNotEmpty))), all);
      case Calc.checked:
        return '${await _count2(Filters.both(f, Filters.cond(key, FilterOp.checked)))}';
      case Calc.unchecked:
        return '${await _count2(Filters.both(f, Filters.cond(key, FilterOp.unchecked)))}';
      case Calc.percentChecked:
        return pct(await _count2(Filters.both(f, Filters.cond(key, FilterOp.checked))), all);
      default:
        break;
    }
    // Min, max, median, range, dates: rows sorted by the property.
    final nf = Filters.both(f, Filters.cond(key, FilterOp.isNotEmpty));
    Future<PropValue?> at(int offset, bool desc) async {
      final r = await vault.run((r) => r.query(
          scope: collection, filter: nf, sorts: [SortSpec(prop: key, descending: desc)], offset: offset, limit: 1));
      if (r.rows.isEmpty) return null;
      return d == null ? null : r.rows.first.valueOf(d);
    }

    String num(double v) => formatNumberValue(v, d == null ? 'number' : d.cfg.numberFormat);
    switch (c) {
      case Calc.min:
        final v = await at(0, false);
        return v == null ? '' : num(v.number);
      case Calc.max:
        final v = await at(0, true);
        return v == null ? '' : num(v.number);
      case Calc.range:
        final a = await at(0, false), b = await at(0, true);
        return a == null || b == null ? '' : num(b.number - a.number);
      case Calc.median:
        final n = await _count2(nf);
        if (n == 0) return '';
        final a = await at((n - 1) ~/ 2, false), b = await at(n ~/ 2, false);
        return num(((a?.number ?? 0) + (b?.number ?? 0)) / 2);
      case Calc.earliest:
        final v = await at(0, false);
        return v == null ? '' : formatDate(v);
      case Calc.latest:
        final v = await at(0, true);
        return v == null ? '' : formatDate(v);
      case Calc.sum || Calc.average:
        final r = await vault.run(
            (r) => r.query(scope: collection, filter: nf, sorts: const [], offset: 0, limit: 1 << 30));
        var s = 0.0;
        for (final row in r.rows) {
          s += row.value(key).number;
        }
        if (c == Calc.sum) return num(s);
        return r.rows.isEmpty ? '' : num(s / r.rows.length);
      default:
        return '';
    }
  }

  void _onVaultClosed() {
    _reload.cancel();
    _gen++;
  }

  @override
  void dispose() {
    vault.removeCloseListener(_onVaultClosed);
    viewSource?.removeListener(_onSourceViews);
    _disposed = true;
    _reload.cancel();
    feed.removeListener(_onChanges);
    super.dispose();
  }
}

String viewKindLabel(ViewKind k) => switch (k) {
      ViewKind.table => 'Table',
      ViewKind.board => 'Board',
      ViewKind.list => 'List',
      ViewKind.gallery => 'Gallery',
      ViewKind.calendar => 'Calendar',
      ViewKind.timeline => 'Timeline',
    };

/// [v] in a number format: number, commas, percent, dollar, euro, pound, yen.
String formatNumberValue(double v, String format) {
  String plain(double x, {int? fixed}) {
    if (fixed != null) return x.toStringAsFixed(fixed);
    if (x == x.roundToDouble() && x.abs() < 1e15) return x.toInt().toString();
    final s = x.toStringAsFixed(6);
    return s.replaceFirst(RegExp(r'0+$'), '').replaceFirst(RegExp(r'\.$'), '');
  }

  String commas(String s) {
    final neg = s.startsWith('-');
    if (neg) s = s.substring(1);
    final parts = s.split('.');
    final b = StringBuffer();
    final w = parts[0];
    for (var i = 0; i < w.length; i++) {
      if (i > 0 && (w.length - i) % 3 == 0) b.write(',');
      b.write(w[i]);
    }
    return '${neg ? '-' : ''}$b${parts.length > 1 ? '.${parts[1]}' : ''}';
  }

  return switch (format) {
    'commas' => commas(plain(v)),
    'percent' => '${plain(v * 100)}%',
    'dollar' => '\$${commas(plain(v, fixed: 2))}',
    'euro' => '€${commas(plain(v, fixed: 2))}',
    'pound' => '£${commas(plain(v, fixed: 2))}',
    'yen' => '¥${commas(plain(v, fixed: 0))}',
    _ => plain(v),
  };
}
