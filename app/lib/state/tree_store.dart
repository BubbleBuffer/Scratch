// The page tree for the desktop sidebar. Children are loaded lazily with
// `list_children`, only for the top level and expanded pages. On a change
// that can move pages (create, delete, move, rename) every loaded list
// reloads; on an edit, only the list that shows the edited page.
//
// Also the sidebar's other state: the favorites (reloaded on every change),
// its width and collapsed flag, and the pages visited in this session
// (home's "Recently visited"). Width, collapse and the expanded pages are
// remembered across runs in [UiState]; visits live in memory.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/vault.dart';
import 'ui_state.dart';

/// One visible row of the tree.
class TreeRow {
  const TreeRow(this.page, this.depth, {this.placeholder = false, this.root});
  final PageRow page;
  final int depth;

  /// The favorite this row is under (null in the tree).
  final String? root;

  /// "No pages inside" under an expanded page without children.
  final bool placeholder;
}

class TreeStore extends ChangeNotifier {
  TreeStore(this.vault, this.feed, {UiState? ui}) : ui = ui ?? UiState() {
    feed.addListener(_onChanges);
    vault.addCloseListener(_onVaultClosed);
  }

  final Vault vault;
  final ChangeFeed feed;

  /// Where width, collapse and expanded pages are remembered.
  final UiState ui;

  /// Children per parent id (null = top level), for loaded parents.
  final Map<String?, List<PageRow>> _children = {};
  final Set<String?> _loading = {};
  /// Expanded pages (remembered in [ui]).
  Set<String> get expanded => ui.expanded;
  final Set<String?> _stale = {};
  bool _staleAll = false;
  final _reload = Debounce();
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;

  static const reloadDelay = Duration(milliseconds: 150);

  // ---- favorites ----

  List<PageRow> favorites = const [];
  bool _favLoading = false, _favAgain = false;

  /// Whether the favorites were loaded at least once.
  bool favoritesLoaded = false;

  /// Reloads the favorites list.
  Future<void> loadFavorites() async {
    if (_inactive) return;
    if (_favLoading) {
      _favAgain = true;
      return;
    }
    _favLoading = true;
    try {
      do {
        _favAgain = false;
        final f = await vault.listFavorites();
        if (_inactive) return;
        favorites = f;
        favoritesLoaded = true;
        notifyListeners();
      } while (_favAgain && !_inactive);
    } catch (e) {
      debugPrint('list_favorites failed: $e');
    } finally {
      _favLoading = false;
    }
  }

  bool isFavorite(String id) => favorites.any((p) => p.id == id);

  Future<void> setFavorite(String id, bool favorite) async {
    await vault.setFavorite(id, favorite);
    if (_inactive) return;
    await loadFavorites();
  }

  // ---- sidebar width and collapse (remembered in [ui]) ----

  static const minWidth = UiState.minSidebarWidth,
      maxWidth = UiState.maxSidebarWidth,
      defaultWidth = UiState.defaultSidebarWidth;

  double get sidebarWidth => ui.sidebarWidth;
  bool get sidebarCollapsed => ui.sidebarCollapsed;

  void setSidebarWidth(double w) {
    final v = w.clamp(minWidth, maxWidth).toDouble();
    if (v == sidebarWidth) return;
    ui.sidebarWidth = v;
    ui.changed();
    notifyListeners();
  }

  void setSidebarCollapsed(bool v) {
    if (v == sidebarCollapsed) return;
    ui.sidebarCollapsed = v;
    ui.changed();
    notifyListeners();
  }

  void toggleSidebar() => setSidebarCollapsed(!sidebarCollapsed);

  // ---- visits (in memory) ----

  /// Page ids opened in this session, most recent first.
  final List<String> visited = [];

  void visit(String id) {
    visited
      ..remove(id)
      ..insert(0, id);
    if (visited.length > 50) visited.removeLast();
    notifyListeners();
  }

  // ---- moves ----

  /// The loaded children of [parent] without starting a load.
  List<PageRow>? loadedChildren(String? parent) => _children[parent];

  /// Moves [id] to the end of [parent]'s children (null = top level).
  Future<void> moveToEnd(String id, String? parent) async {
    final kids = await vault.listChildren(parent);
    final others = [for (final k in kids) if (k.id != id) k];
    await vault.movePage(id, newParent: parent, after: others.isEmpty ? null : others.last.id);
    if (parent != null) expand(parent);
  }

  /// Page ids below [id] (its whole subtree, loaded from the core).
  Future<Set<String>> descendants(String id) async {
    final out = <String>{};
    final todo = [id];
    while (todo.isNotEmpty) {
      final p = todo.removeLast();
      for (final k in await vault.listChildren(p)) {
        if (out.add(k.id)) todo.add(k.id);
      }
    }
    return out;
  }

  /// Loaded children of [parent], or null (and a load starts) if not loaded.
  List<PageRow>? childrenOf(String? parent) {
    final c = _children[parent];
    if (c == null) _load(parent);
    return c;
  }

  Future<void> _load(String? parent) async {
    if (_inactive) return;
    if (!_loading.add(parent)) return;
    try {
      final rows = await vault.listChildren(parent);
      if (_inactive) return;
      _children[parent] = rows;
      notifyListeners();
    } catch (e) {
      debugPrint('list_children failed: $e');
    } finally {
      _loading.remove(parent);
    }
  }

  /// Whether [id] is expanded in the tree (or, with [fav], in Favorites).
  bool isExpanded(String id, {bool fav = false}) => (fav ? ui.favExpanded : expanded).contains(id);

  void toggle(String id, {bool fav = false}) =>
      isExpanded(id, fav: fav) ? collapse(id, fav: fav) : expand(id, fav: fav);

  void expand(String id, {bool fav = false}) {
    if ((fav ? ui.favExpanded : expanded).add(id)) {
      ui.changed();
      childrenOf(id);
      notifyListeners();
    }
  }

  void collapse(String id, {bool fav = false}) {
    if ((fav ? ui.favExpanded : expanded).remove(id)) {
      ui.changed();
      notifyListeners();
    }
  }

  /// The rows the sidebar shows: top level, then children of expanded pages.
  List<TreeRow> visibleRows() {
    final out = <TreeRow>[];
    _walk(out, childrenOf(null), 0, expanded, null);
    return out;
  }

  /// The Favorites rows: each favorite, then the children of the ones
  /// expanded there.
  List<TreeRow> favoriteRows() {
    final out = <TreeRow>[];
    for (final f in favorites) {
      _walk(out, [f], 0, ui.favExpanded, f.id);
    }
    return out;
  }

  void _walk(List<TreeRow> out, List<PageRow>? kids, int depth, Set<String> open, String? root) {
    if (kids == null) return;
    for (final p in kids) {
      out.add(TreeRow(p, depth, root: root));
      if (open.contains(p.id) && depth < 32) {
        final sub = childrenOf(p.id);
        if (sub != null && sub.isEmpty) {
          out.add(TreeRow(p, depth + 1, placeholder: true, root: root));
        } else {
          _walk(out, sub, depth + 1, open, root);
        }
      }
    }
  }

  /// Reloads every loaded list now.
  Future<void> refresh() async {
    if (_inactive) return;
    final parents = _children.keys.toList();
    await Future.wait(parents.map(_reloadList));
  }

  Future<void> _reloadList(String? parent) async {
    if (_inactive) return;
    try {
      final rows = await vault.listChildren(parent);
      if (_inactive) return;
      _children[parent] = rows;
      notifyListeners();
    } catch (_) {
      // The parent is gone (deleted or moved to the trash).
      if (_inactive) return;
      _children.remove(parent);
      if (parent != null && expanded.remove(parent)) ui.changed();
      notifyListeners();
    }
  }

  void _onChanges(ChangeBatch b) {
    if (_inactive) return;
    loadFavorites();
    if (b.treeChanged) {
      _staleAll = true;
    } else {
      for (final id in b.changedPages) {
        for (final e in _children.entries) {
          if (e.value.any((r) => r.id == id)) _stale.add(e.key);
        }
      }
      if (_stale.isEmpty) return;
    }
    _reload.run(reloadDelay, () {
      final parents = _staleAll ? _children.keys.toList() : _stale.toList();
      _staleAll = false;
      _stale.clear();
      for (final p in parents) {
        _reloadList(p);
      }
    });
  }

  void _onVaultClosed() {
    _reload.cancel();
    _favAgain = false;
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
