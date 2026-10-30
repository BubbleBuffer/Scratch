// The type filter chips of the search panel: All, Pages and one per registered
// source, plus a removable chip for each other `type:` value in the query.
// The pills are bb_ui's; this file decides which chips a query shows.

import 'package:bb_ui/search.dart' as bb;
import 'package:flutter/widgets.dart';

import 'addons.dart';
import 'search_groups.dart';

/// The label of a dynamic type chip: the plural of a known type, else the
/// value as typed.
String typeChipLabel(String value) => switch (value) {
  'document' => 'Documents',
  'database' => 'Databases',
  _ =>
    appAddonContext.searchSources.all
            .where((source) => source.noteTypes.contains(value))
            .firstOrNull
            ?.label ??
        value,
};

/// The type filter chips. All, Pages and Chats are fixed: each is selected
/// when the query's type values are exactly its set. A query whose type
/// values match no fixed set gets one selected, removable chip per value
/// (`type:document` shows "Documents"). [onPick] receives the type values to
/// write (empty for All).
class SearchTypeChips extends StatelessWidget {
  const SearchTypeChips({
    super.key,
    required this.active,
    required this.onPick,
  });
  final Set<String> active;
  final ValueChanged<Set<String>> onPick;

  @override
  Widget build(BuildContext context) {
    final fixed = <(String, String, Set<String>)>[
      ('all', 'All', const {}),
      ('pages', 'Pages', searchTypePages),
      for (final source in appAddonContext.searchSources.all)
        if (source.noteTypes.isNotEmpty)
          (source.groupId, source.label, source.noteTypes),
    ];
    final matchesFixed = fixed.any((f) => _sameTypes(active, f.$3));
    final dynamicValues = matchesFixed
        ? const <String>[]
        : (active.toList()..sort());
    return bb.SearchTypeChips(
      chips: [
        for (final (id, label, types) in fixed)
          bb.SearchTypeChip(
            id: id,
            label: label,
            selected: types.isEmpty
                ? active.isEmpty
                : _sameTypes(active, types),
            onTap: () {
              final selected = types.isEmpty
                  ? active.isEmpty
                  : _sameTypes(active, types);
              // Tapping the selected chip returns to All.
              onPick(selected ? const {} : types);
            },
          ),
        for (final value in dynamicValues)
          bb.SearchTypeChip(
            id: 'value-$value',
            label: typeChipLabel(value),
            selected: true,
            removable: true,
            // Removing a dynamic chip drops only its own value.
            onTap: () => onPick({
              for (final v in active)
                if (v != value) v,
            }),
          ),
      ],
    );
  }
}

bool _sameTypes(Set<String> a, Set<String> b) =>
    a.length == b.length && a.containsAll(b);
