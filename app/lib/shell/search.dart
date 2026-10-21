// Search through the core's full-text index (`vault.search`). On desktop the
// sidebar's Search row, Ctrl+K and Ctrl+P open the search overlay
// ([showSearchOverlay]) over whatever is open. Mobile opens [SearchScreen].
// Results are grouped Pages, then Chats; each row shows its path or breadcrumb
// and, for selective queries, matching text. The type chips write the `type:`
// operator into the query. Choosing a page opens it; choosing a chat opens the
// chat at its stored message range, or the chat itself for a title-only row.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/vault.dart';
import '../registry/addon_registry.dart';
import 'addons.dart';
import '../state/app_state.dart';
import '../ui/ui.dart';
import 'navigation.dart';
import 'search_groups.dart';
import 'search_widgets.dart';

export '../addons/agent/api.dart' show openChatHit;

export 'search_widgets.dart' show SearchResultRow, pagePath;

/// Bumped to ask home to focus its search field.
final searchRequest = ValueNotifier<int>(0);

/// The results of the latest query; stale answers are dropped.
///
/// Page-only unless [includeExternal] is set: home's inline search shows
/// pages, and the overlay and the mobile screen also show the chats.
class PageSearch extends ChangeNotifier {
  PageSearch(this.vault, {this.includeExternal = false, this.addons});
  final AddonContext? addons;
  AddonContext get _addons => addons ?? appAddonContext;
  final Vault vault;

  /// Whether hits from other sources (chats) are kept.
  final bool includeExternal;

  String _query = '';
  List<Hit>? _hits;
  bool _narrowed = false;
  int _gen = 0;
  bool _disposed = false;
  Set<String> _types = const {};
  List<String> _terms = const [];

  String get query => _query;

  /// Results for [query], or null when the query is empty.
  List<Hit>? get hits => _hits;

  /// Broad queries prioritize titles and fill with unranked text matches.
  bool get narrowed => _narrowed;

  /// The `type:` values the query filters on (empty: every type).
  Set<String> get activeTypes => _types;

  /// The words to highlight: the query's text, or the corrected words when
  /// every visible result used the correction.
  List<String> get terms {
    final corrected = correction;
    return corrected == null ? _terms : highlightTerms([corrected]);
  }

  /// Correction is explained only when every visible result used it.
  String? get correction {
    final hits = _hits;
    if (hits == null || hits.isEmpty || hits.any((hit) => !hit.fuzzy)) {
      return null;
    }
    return hits.first.corrected;
  }

  Future<void> run(String query) async {
    _query = query;
    final gen = ++_gen;
    if (query.trim().isEmpty) {
      _hits = null;
      _narrowed = false;
      _types = const {};
      _terms = const [];
      notifyListeners();
      return;
    }
    try {
      ({Set<String> types, List<String> terms})? reading;
      var parsedReady = false;
      final parsing = _parse(query).then((value) {
        reading = value;
        parsedReady = true;
      });
      final List<Hit> hits;
      var narrowed = false;
      if (includeExternal &&
          _addons.searchSources.all.any((source) => source.indexed)) {
        final results = await vault.run(
          (v) => v.searchAdaptive(query: query, limit: 100),
        );
        hits = results.hits
            .where(
              (hit) =>
                  hit.source == 'vault' ||
                  _addons.searchSources.of(hit.source)?.indexed == true,
            )
            .toList();
        narrowed = results.narrowed;
      } else {
        // The bridge scopes page search before ranking and the limit.
        hits = (await vault.search(query, limit: 100)).toList();
      }
      if (includeExternal) {
        final extra = await Future.wait([
          for (final source in _addons.searchSources.all)
            if (!source.indexed) _sourceHits(source, query),
        ]);
        for (final rows in extra) {
          hits.addAll(rows);
        }
      }
      if (gen != _gen || _disposed) return;
      _hits = hits;
      _narrowed = narrowed;
      final appliedReading = parsedReady;
      if (appliedReading) {
        _types = reading?.types ?? const {};
        _terms = reading?.terms ?? const [];
      }
      notifyListeners();
      // Preserve the first results frame even when optional chip metadata
      // takes longer than the search itself.
      if (!appliedReading) {
        await parsing;
        if (gen != _gen || _disposed) return;
        _types = reading?.types ?? const {};
        _terms = reading?.terms ?? const [];
        notifyListeners();
      }
    } catch (e) {
      debugPrint('search failed: $e');
    }
  }

  Future<List<Hit>> _sourceHits(AddonSearchSource source, String query) async {
    try {
      return [
        for (final row in await source.search(query)) row.toHit(source.id),
      ];
    } catch (e) {
      debugPrint('search source ${source.id} failed: $e');
      return const [];
    }
  }

  /// The chips' and highlights' reading of [query]. A failed parse only
  /// loses the chip state and highlights; the results still show.
  Future<({Set<String> types, List<String> terms})?> _parse(
    String query,
  ) async {
    try {
      final parsed = await vault.run((v) => v.parseSearch(q: query));
      final texts = <String>[
        for (final token in parsed.tokens)
          if (token.kind.kind == SearchTokenKind.text ||
              token.kind.kind == SearchTokenKind.phrase)
            token.kind.value,
      ];
      final types = typeValuesOf(typeFiltersOf(query, parsed));
      return (types: types, terms: highlightTerms(texts));
    } catch (e) {
      debugPrint('search parse failed: $e');
      return null;
    }
  }

  /// [text] with its positive `type:` operators replaced by [types] (empty
  /// removes them). It parses [text] now, so a chip click rewrites exactly
  /// what the field shows. Null when the parse fails.
  Future<String?> rewriteTypes(String text, Set<String> types) async {
    try {
      final parsed = await vault.run((v) => v.parseSearch(q: text));
      return withTypeFilters(text, typeFiltersOf(text, parsed), types);
    } catch (e) {
      debugPrint('search type rewrite failed: $e');
      return null;
    }
  }

  /// Runs the current query again (after the pages changed).
  Future<void> refresh() => run(_query);

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// Sources own navigation and opaque targets; vault pages use host navigation.
Future<void> openSearchHit(
  BuildContext context,
  Hit hit, {
  AddonContext? addons,
}) async {
  if (hit.source == 'vault') {
    await openPage(context, hit.id, hit.title);
  } else {
    final source = (addons ?? appAddonContext).searchSources.of(hit.source);
    await source?.open(context, AddonSearchResult.fromHit(hit));
  }
}

/// Mobile search: a full-screen page with the field and type chips at the
/// top and the grouped results below.
class SearchScreen extends StatefulWidget {
  const SearchScreen({super.key, this.initialQuery});

  /// Starts with this query in the field and its results shown. Form B's
  /// "Open in Search" passes the chip's query.
  final String? initialQuery;

  @override
  State<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends State<SearchScreen> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  late final PageSearch _query = PageSearch(
    context.app.vault,
    includeExternal: true,
  )..addListener(_changed);

  @override
  void initState() {
    super.initState();
    _startSearch(_ctl, _query.run, widget.initialQuery);
  }

  @override
  void dispose() {
    _query.dispose();
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  Future<void> _setTypes(Set<String> types) async {
    final text = _ctl.text;
    final next = await _query.rewriteTypes(text, types);
    // The field changed while the parse ran: the newer text wins.
    if (!mounted || next == null || _ctl.text != text) return;
    _ctl.value = TextEditingValue(
      text: next,
      selection: TextSelection.collapsed(offset: next.length),
    );
    _query.run(next);
  }

  /// A page opens in place; a chat closes this screen and opens the chat.
  void _pick(Hit hit) {
    if (hit.source == 'vault') {
      openPage(context, hit.id, hit.title);
      return;
    }
    final navigator = Navigator.of(context);
    final host = navigator.context;
    unawaited(navigator.maybePop());
    unawaited(openSearchHit(host, hit));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final mq = MediaQuery.of(context);
    final hits = _query.hits;
    Widget note(String s) => Padding(
      padding: const EdgeInsets.all(Layout.gutter),
      child: Text(s, style: ts(ty.ui, c.textTertiary, weight: 400)),
    );
    return ColoredBox(
      color: c.bgPrimary,
      child: Padding(
        padding: EdgeInsets.only(top: mq.padding.top),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(Layout.gutter - 4, 8, 6, 8),
              child: Row(
                children: [
                  Expanded(
                    child: SearchField(
                      fieldKey: const ValueKey('search-field'),
                      controller: _ctl,
                      focusNode: _focus,
                      onChanged: _query.run,
                      placeholder: 'Search pages and chats',
                    ),
                  ),
                  TextBtn(
                    'Cancel',
                    key: const ValueKey('search-cancel'),
                    height: 40,
                    color: c.textSecondary,
                    onTap: () => Navigator.of(context).maybePop(),
                  ),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(
                Layout.gutter,
                0,
                Layout.gutter,
                8,
              ),
              child: SearchTypeChips(
                active: _query.activeTypes,
                onPick: (types) => unawaited(_setTypes(types)),
              ),
            ),
            Expanded(
              child: hits == null
                  ? note('Search page titles, text and chats.')
                  : hits.isEmpty
                  ? note('No results')
                  : SearchResultsList(
                      sections: groupSearchHits(hits),
                      mobile: true,
                      terms: _query.terms,
                      padding: EdgeInsets.only(bottom: mq.padding.bottom + 8),
                      onPick: _pick,
                    ),
            ),
          ],
        ),
      ),
    );
  }
}

bool _overlayOpen = false;

/// Puts [query] in the field and runs it, for a search opened with one.
void _startSearch(
  TextEditingController ctl,
  void Function(String) run,
  String? query,
) {
  final text = query?.trim() ?? '';
  if (text.isEmpty) return;
  ctl.value = TextEditingValue(
    text: text,
    selection: TextSelection.collapsed(offset: text.length),
  );
  run(text);
}

/// Opens the desktop search overlay (once; a second call does nothing while
/// it is open). Choosing a page opens it in place of the stack; choosing a
/// chat opens that chat.
Future<void> showSearchOverlay(BuildContext context, {String? query}) async {
  if (_overlayOpen) return;
  _overlayOpen = true;
  try {
    final hit = await showModalPanel<Hit>(
      context,
      (_) => SearchPanel(initialQuery: query),
      width: 640,
      height: 520,
    );
    if (hit != null && context.mounted) {
      await openSearchHit(context, hit);
    }
  } finally {
    _overlayOpen = false;
  }
}

/// The desktop search overlay. The body does the field, chips, grouped
/// results and keyboard selection; this owns the query and closes with the
/// picked hit.
class SearchPanel extends StatefulWidget {
  const SearchPanel({super.key, this.initialQuery});

  /// Starts with this query in the field and its results shown.
  final String? initialQuery;

  @override
  State<SearchPanel> createState() => _SearchPanelState();
}

class _SearchPanelState extends State<SearchPanel> {
  @override
  void initState() {
    super.initState();
    _startSearch(_ctl, _query.run, widget.initialQuery);
  }

  final _ctl = TextEditingController();
  late final PageSearch _query = PageSearch(
    context.app.vault,
    includeExternal: true,
  )..addListener(_changed);

  @override
  void dispose() {
    _query.dispose();
    _ctl.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  @override
  Widget build(BuildContext context) => SearchPanelBody(
    controller: _ctl,
    onQueryChanged: _query.run,
    onPick: (hit) => Navigator.of(context).pop(hit),
    hits: _query.hits,
    rewriteTypes: _query.rewriteTypes,
    narrowed: _query.narrowed,
    correction: _query.correction,
    terms: _query.terms,
    activeTypes: _query.activeTypes,
    mixed: true,
  );
}
