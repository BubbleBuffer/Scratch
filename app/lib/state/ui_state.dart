// What the interface remembers on this device between runs: the sidebar's
// width, collapse and expanded pages, the page that was open, and which
// toggles are open on each page. Kept in `<data dir>/ui_state.json`, beside
// settings.json, and never synced. Per-page display options (small text,
// full width, lock, cover position) are not here: they live in the page and
// sync with it (see PagesStore.prefs).
//
// The file belongs to one vault: when the vault folder changes, the
// page-specific parts start empty. Writes are debounced; [flush] writes now.

import '../core/debounce.dart';

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

class UiState extends ChangeNotifier {
  UiState({this.file, this.vaultRoot});

  /// Where the state is saved (null: in memory only, as in most tests).
  final File? file;

  /// The vault this state belongs to.
  final String? vaultRoot;

  static const minSidebarWidth = 200.0,
      maxSidebarWidth = 480.0,
      defaultSidebarWidth = 240.0;
  static const minAgentPanelWidth = 320.0,
      maxAgentPanelWidth = 560.0,
      defaultAgentPanelWidth = 400.0;

  /// Pages whose toggle state is kept (the most recently changed).
  static const maxToggledPages = 200;

  /// Expanded sidebar pages kept at most.
  static const maxExpanded = 500;

  static const saveDelay = Duration(milliseconds: 300);

  double sidebarWidth = defaultSidebarWidth;
  bool sidebarCollapsed = false;
  bool captureTask = true;
  bool allPagesExpanded = false;
  bool agentPanelOpen = false;
  double agentPanelWidth = defaultAgentPanelWidth;
  String? agentSessionId;

  /// Expanded pages in the sidebar tree.
  final Set<String> expanded = {};

  /// Expanded pages in the sidebar's Favorites (apart from the tree).
  final Set<String> favExpanded = {};

  /// The page open when the app last ran (null: home).
  String? lastPage;

  /// Open toggle block ids per page, least recently changed first.
  final LinkedHashMap<String, Set<String>> _toggles = LinkedHashMap();

  Timer? _timer;

  /// Loads [file] for the vault at [vaultRoot]; a missing or unreadable file
  /// gives the defaults.
  static UiState load(File file, String vaultRoot) {
    final s = UiState(file: file, vaultRoot: vaultRoot);
    try {
      if (file.existsSync()) {
        s._fromJson(
          jsonDecode(file.readAsStringSync()) as Map<String, dynamic>,
        );
      }
    } catch (e) {
      debugPrint('ui state unreadable: $e');
    }
    return s;
  }

  void _fromJson(Map<String, dynamic> j) {
    final w = (j['sidebarWidth'] as num?)?.toDouble();
    if (w != null) {
      sidebarWidth = w.clamp(minSidebarWidth, maxSidebarWidth).toDouble();
    }
    sidebarCollapsed = (j['sidebarCollapsed'] as bool?) ?? false;
    if (j['vault'] != vaultRoot) return;
    captureTask = (j['captureTask'] as bool?) ?? true;
    allPagesExpanded = (j['allPagesExpanded'] as bool?) ?? false;
    agentPanelOpen = (j['agentPanelOpen'] as bool?) ?? false;
    final panelWidth = (j['agentPanelWidth'] as num?)?.toDouble();
    if (panelWidth != null && panelWidth.isFinite) {
      agentPanelWidth = panelWidth
          .clamp(minAgentPanelWidth, maxAgentPanelWidth)
          .toDouble();
    }
    agentSessionId = j['agentSessionId'] as String?;
    expanded.addAll([
      for (final e in (j['expanded'] as List?) ?? const []) e as String,
    ]);
    favExpanded.addAll([
      for (final e in (j['favExpanded'] as List?) ?? const []) e as String,
    ]);
    lastPage = j['lastPage'] as String?;
    final t = (j['toggles'] as Map<String, dynamic>?) ?? const {};
    for (final e in t.entries) {
      _toggles[e.key] = {for (final b in e.value as List) b as String};
    }
  }

  Map<String, dynamic> toJson() => {
    'vault': ?vaultRoot,
    'sidebarWidth': sidebarWidth,
    'sidebarCollapsed': sidebarCollapsed,
    'captureTask': captureTask,
    'allPagesExpanded': allPagesExpanded,
    'agentPanelOpen': agentPanelOpen,
    'agentPanelWidth': agentPanelWidth,
    'agentSessionId': ?agentSessionId,
    'expanded': expanded.take(maxExpanded).toList(),
    'favExpanded': favExpanded.take(maxExpanded).toList(),
    'lastPage': ?lastPage,
    'toggles': {
      for (final e in _toggles.entries)
        if (e.value.isNotEmpty) e.key: e.value.toList(),
    },
  };

  /// The open toggles of [pageId]. The set is live: adding or removing ids
  /// saves the state.
  Set<String> openToggles(String pageId) => _TogglesSet(this, pageId);

  Set<String> _togglesOf(String pageId) => _toggles[pageId] ?? const {};

  void _setToggle(String pageId, String blockId, bool open) {
    final s = _toggles.remove(pageId) ?? <String>{};
    if (open) {
      s.add(blockId);
    } else {
      s.remove(blockId);
    }
    if (s.isNotEmpty) _toggles[pageId] = s; // re-inserted: most recent last
    while (_toggles.length > maxToggledPages) {
      _toggles.remove(_toggles.keys.first);
    }
    changed();
  }

  /// Schedules a save.
  void changed() {
    notifyListeners();
    if (file == null) return;
    _timer?.cancel();
    _timer = Timer(Debounce.testDelay(saveDelay), flush);
  }

  /// Writes the state now (atomically: a temporary file, then a rename).
  void flush() {
    _timer?.cancel();
    _timer = null;
    final f = file;
    if (f == null) return;
    try {
      f.parent.createSync(recursive: true);
      final tmp = File('${f.path}.tmp');
      tmp.writeAsStringSync(jsonEncode(toJson()), flush: true);
      tmp.renameSync(f.path);
    } catch (e) {
      debugPrint('ui state not saved: $e');
    }
  }

  @override
  void dispose() {
    if (_timer != null) flush();
    super.dispose();
  }
}

/// One page's open toggles, writing through to [UiState].
class _TogglesSet extends SetBase<String> {
  _TogglesSet(this.ui, this.pageId);
  final UiState ui;
  final String pageId;

  Set<String> get _s => ui._togglesOf(pageId);

  @override
  bool add(String value) {
    if (_s.contains(value)) return false;
    ui._setToggle(pageId, value, true);
    return true;
  }

  @override
  bool remove(Object? value) {
    if (value is! String || !_s.contains(value)) return false;
    ui._setToggle(pageId, value, false);
    return true;
  }

  @override
  bool contains(Object? element) => _s.contains(element);

  @override
  String? lookup(Object? element) => _s.lookup(element);

  @override
  Iterator<String> get iterator => _s.toList().iterator;

  @override
  int get length => _s.length;

  @override
  Set<String> toSet() => _s.toSet();
}
