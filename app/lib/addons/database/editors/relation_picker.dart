// Relations: linked page titles in cells (resolved and cached by id) and
// the picker that links and unlinks rows of the related database.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../core/vault.dart';
import '../../../registry/property_registry.dart';
import '../../../state/app_state.dart';
import '../../../state/pages_store.dart';
import '../../../ui/ui.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';

/// The name of linked page [id]: its [title], else the first line of its
/// body (stored meta, see [PagesStore.meta]), else "Untitled".
String _linkedName(PagesStore pages, String id, String title) =>
    displayTitle(title, firstLine: title.trim().isEmpty ? pages.meta(id)?.firstLine ?? '' : '');

/// Titles and icons of linked pages, by id.
class RelationTitles {
  static final _cache = <String, (String, String?)>{};
  static final _loading = <String, Future<void>>{};
  static final changed = ValueNotifier<int>(0);

  static (String, String?)? of(String id) => _cache[id];

  static void remember(PageRow r) => _cache[r.id] = (r.title, r.icon);

  static void load(Vault vault, Iterable<String> ids) {
    for (final id in ids) {
      if (_cache.containsKey(id) || _loading.containsKey(id)) continue;
      _loading[id] = vault.loadPage(id).then((p) {
        _cache[id] = (p.title, p.icon);
        changed.value++;
      }, onError: (Object _) {
        _cache[id] = ('Deleted page', null);
      }).whenComplete(() => _loading.remove(id));
    }
  }
}

/// Linked pages as underlined titles.
class RelationCell extends StatelessWidget {
  const RelationCell({super.key, required this.ids, required this.style, this.wrap = false});
  final List<String> ids;
  final TextStyle style;
  final bool wrap;

  @override
  Widget build(BuildContext context) {
    RelationTitles.load(context.app.vault, ids);
    final pages = context.watch<PagesStore>();
    return ValueListenableBuilder<int>(
      valueListenable: RelationTitles.changed,
      builder: (context, _, _) {
        final c = context.colors;
        final items = [
          for (final id in ids)
            Row(mainAxisSize: MainAxisSize.min, children: [
              PageIcon(icon: RelationTitles.of(id)?.$2, size: style.fontSize! + 2, color: c.iconSecondary),
              const SizedBox(width: 4),
              Flexible(
                child: Text(
                  _linkedName(pages, id, RelationTitles.of(id)?.$1 ?? ''),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: style.copyWith(
                    decoration: TextDecoration.underline,
                    decorationColor: c.divider,
                  ),
                ),
              ),
            ]),
        ];
        return wrap
            ? Wrap(spacing: 8, runSpacing: 4, children: items)
            : ClipRect(child: Row(children: [for (final w in items) Padding(padding: const EdgeInsets.only(right: 8), child: w)]));
      },
    );
  }
}

/// The related database of [def]: its setting, else [fallback] (the
/// property's own database).
String relationTarget(PropDef def, String fallback) => PropConfig(def.config).relationTo ?? fallback;

Future<void> editRelation(EditArgs a, String collection) =>
    showPanel<void>(a.context, a.anchor, (ctx) => _RelationPicker(args: a, collection: collection), width: 320);

class _RelationPicker extends StatefulWidget {
  const _RelationPicker({required this.args, required this.collection});
  final EditArgs args;
  final String collection;

  @override
  State<_RelationPicker> createState() => _RelationPickerState();
}

class _RelationPickerState extends State<_RelationPicker> {
  late List<String> ids = [...widget.args.value.ids];
  List<PageRow> rows = const [];
  String query = '';
  int _gen = 0;

  @override
  void initState() {
    super.initState();
    _search();
  }

  Future<void> _search() async {
    final g = ++_gen;
    final r = await context.app.vault.run((v) => v.query(
        scope: widget.collection,
        filter: query.isEmpty ? null : Filters.cond(BuiltinKeys.title, FilterOp.contains, Values.text(query)),
        sorts: const [],
        offset: 0,
        limit: 50));
    if (!mounted || g != _gen) return;
    for (final row in r.rows) {
      RelationTitles.remember(row);
    }
    setState(() => rows = r.rows);
  }

  void _toggle(String id) {
    setState(() => ids.contains(id) ? ids.remove(id) : ids.add(id));
    widget.args.onChanged(Values.relation(ids));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final pages = context.watch<PagesStore>();
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      MenuField(
        key: const ValueKey('relation-search'),
        placeholder: 'Link or create a page…',
        autofocus: !context.mobile,
        onChanged: (s) {
          query = s.trim();
          _search();
        },
      ),
      if (ids.isNotEmpty) ...[
        const MenuHeader('Linked'),
        for (final id in ids)
          MenuItem(
            label: _linkedName(pages, id, RelationTitles.of(id)?.$1 ?? ''),
            leading: PageIcon(icon: RelationTitles.of(id)?.$2, size: 18, color: c.iconSecondary),
            trailing: Icon20(Ico.close, size: 14, color: c.iconTertiary),
            onTap: () => _toggle(id),
          ),
      ],
      const MenuHeader('Pages'),
      ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 280),
        child: ListView(shrinkWrap: true, children: [
          for (final r in rows)
            if (!ids.contains(r.id) && r.id != widget.args.value.text)
              MenuItem(
                key: ValueKey('relation-${r.title}'),
                label: _linkedName(pages, r.id, r.title),
                leading: PageIcon(icon: r.icon, size: 18, color: c.iconSecondary),
                onTap: () => _toggle(r.id),
              ),
        ]),
      ),
    ]);
  }
}
