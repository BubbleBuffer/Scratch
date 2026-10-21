// Search result groups and the parts of a result the UI reads without
// drawing: which group a hit belongs to, where a chat hit opens, and the
// `type:` operator the type chips write into the query. Matching and ranking
// stay in the core; this only orders what the core returned.

import 'dart:convert';

import '../core/models.dart';
import '../registry/addon_registry.dart';
import 'addons.dart';
export '../addons/agent/api.dart' show ChatSearchTarget;

/// A group's stable widget key and display label, supplied by its source.
class SearchGroup {
  const SearchGroup(this.name, this.label);
  final String name, label;
  static const pages = SearchGroup('pages', 'Pages');
}

bool isExternalHit(Hit hit) => hit.source != 'vault';

SearchGroup searchGroupOf(
  Hit hit, {
  ExtensionRegistry<AddonSearchSource>? sources,
}) {
  final source = (sources ?? appAddonContext.searchSources).of(hit.source);
  return source == null
      ? SearchGroup.pages
      : SearchGroup(source.groupId, source.label);
}

String hitRowId(Hit hit) =>
    hit.source == 'vault' ? hit.id : '${hit.source}/${hit.id}';

class SearchSection {
  const SearchSection(this.group, this.hits);
  final SearchGroup group;
  final List<Hit> hits;
  String get label => group.label;
}

/// Pages first, followed by registered sources in registration order. Each
/// source keeps the rank order supplied by the index or its query callback.
List<SearchSection> groupSearchHits(
  List<Hit> hits, {
  ExtensionRegistry<AddonSearchSource>? sources,
}) {
  final registry = sources ?? appAddonContext.searchSources;
  final pages = hits.where((hit) => hit.source == 'vault').toList();
  return [
    if (pages.isNotEmpty) SearchSection(SearchGroup.pages, pages),
    for (final source in registry.all)
      if (hits.any((hit) => hit.source == source.id))
        SearchSection(
          SearchGroup(source.groupId, source.label),
          hits.where((hit) => hit.source == source.id).toList(),
        ),
  ];
}

List<Hit> flattenSections(List<SearchSection> sections) => [
  for (final section in sections) ...section.hits,
];

/// The type chips' operator values. Pages covers both page types; the query
/// text carries them as `type:document,database`.
const searchTypePages = {'document', 'database'};
const searchTypeChats = {'chat'};

/// A positive `type:` operator in a query: where it sits in the query's UTF-8
/// bytes (end exclusive, as the core counts them) and the values it names.
class TypeFilter {
  const TypeFilter({
    required this.start,
    required this.end,
    required this.values,
  });
  final int start;
  final int end;
  final List<String> values;
}

/// The positive `type:` operators of [parsed], the parse of [query]. The
/// parser names a built-in operator by its normalized key, so `Ty_pe:` and
/// `t-y-p-e:` are `type` too. A negated operator (`-type:chat`) is not a
/// filter: the spec's `negated` flag says so. A quoted key (`"type":chat`) is
/// never a built-in operator, and a phrase such as `"say type:chat"` is a
/// phrase token, so neither reaches here.
///
/// The parser emits one operator token and one spec operator per built-in
/// operator, in the same order, so the two lists are paired by position. If
/// the counts ever disagree, no operator is read as a filter.
List<TypeFilter> typeFiltersOf(String query, ParsedQuery parsed) {
  final bytes = utf8.encode(query);
  final tokens = [
    for (final t in parsed.tokens)
      if (t.kind.kind == SearchTokenKind.operator_ && t.kind.key == 'type') t,
  ];
  final ops = [
    for (final op in parsed.spec.operators)
      if (op.key == 'type') op,
  ];
  if (tokens.length != ops.length) return const [];
  final filters = <TypeFilter>[];
  for (var i = 0; i < tokens.length; i++) {
    final token = tokens[i];
    final op = ops[i];
    if (op.negated || op.property) continue;
    if (token.start < 0 ||
        token.end > bytes.length ||
        token.start >= token.end) {
      continue;
    }
    filters.add(
      TypeFilter(
        start: token.start,
        end: token.end,
        values: [
          for (final v in op.value.split(','))
            if (v.trim().isNotEmpty) v.trim().toLowerCase(),
        ],
      ),
    );
  }
  return filters;
}

/// The type values [filters] name, each comma-separated value on its own.
Set<String> typeValuesOf(Iterable<TypeFilter> filters) => {
  for (final f in filters) ...f.values,
};

bool _isSpace(int byte) =>
    byte == 0x20 || byte == 0x09 || byte == 0x0a || byte == 0x0d;

/// [query] with the [filters] spans cut out (their bytes, and the whitespace
/// beside them) and [types] written in their place. Empty [types] removes the
/// filters. Everything else, including quoted phrases, other operators and
/// negated or quoted-key tokens, is kept byte for byte.
String withTypeFilters(
  String query,
  List<TypeFilter> filters,
  Set<String> types,
) {
  final bytes = utf8.encode(query);
  final out = <int>[];
  var at = 0;
  final sorted = [...filters]..sort((a, b) => a.start.compareTo(b.start));
  for (final f in sorted) {
    var start = f.start;
    var end = f.end;
    // Take the space after the operator, else the space before it.
    var after = end;
    while (after < bytes.length && _isSpace(bytes[after])) {
      after++;
    }
    if (after > end) {
      end = after;
    } else {
      while (start > at && _isSpace(bytes[start - 1])) {
        start--;
      }
    }
    if (start < at) start = at;
    out.addAll(bytes.sublist(at, start));
    at = end;
  }
  out.addAll(bytes.sublist(at));
  final stripped = utf8.decode(out).trim();
  if (types.isEmpty) return stripped;
  final op = 'type:${types.join(',')}';
  return stripped.isEmpty ? op : '$stripped $op';
}

/// The words to highlight in a result: the query's text and phrases, or the
/// corrected words for a typo-corrected hit. Lower-case, longest first.
List<String> highlightTerms(Iterable<String> texts) {
  final terms = {
    for (final t in texts)
      for (final part in t.toLowerCase().split(RegExp(r'\s+')))
        if (part.isNotEmpty) part,
  }.toList();
  terms.sort((a, b) => b.length.compareTo(a.length));
  return terms;
}
