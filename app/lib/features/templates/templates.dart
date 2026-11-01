// Page templates. Any page can hold templates for its new child pages (the
// core keeps them as children marked as templates, left out of lists and
// search), and one of them can be the default, which every new child
// starts from.
//
// - An empty page shows one row of starters under the title ([PageStarters]:
//   "Blank", one chip per template of its parent, "+ Template");
//   picking a template copies it into the page.
// - An open template says so in a banner, with "Set as default" and "Turn
//   into page".
// - A database's "New ▾" menu lists its templates (showTemplateMenu).
//
// The page editor reaches this file through [withTemplates], wrapped
// around the property rows of the page top (lib/addons/database/
// page_properties.dart), and through [PageStarters] on an empty page.

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../shell/navigation.dart' as nav;
import '../../state/app_state.dart';
import '../../state/pages_store.dart' show firstLineOfTexts;
import '../../ui/ui.dart';
import '../../ui/database/db_icons.dart';
import '../../ui/database/widgets.dart';

/// [child] (the top of page [pageId]) with the banner of a template.
Widget withTemplates({
  required String pageId,
  required String? parent,
  required double left,
  required double width,
  required Widget child,
}) => parent == null
    ? child
    : _TemplateTop(
        key: ValueKey('templates-$pageId'),
        pageId: pageId,
        parent: parent,
        left: left,
        width: width,
        child: child,
      );

class _TemplateTop extends StatefulWidget {
  const _TemplateTop({
    super.key,
    required this.pageId,
    required this.parent,
    required this.left,
    required this.width,
    required this.child,
  });
  final String pageId, parent;
  final double left, width;
  final Widget child;

  @override
  State<_TemplateTop> createState() => _TemplateTopState();
}

class _TemplateTopState extends State<_TemplateTop> {
  List<PageRow> _templates = const [];
  String? _default;
  String _parentTitle = '';
  String _parentLine = '';
  ChangeFeed? _feed;

  bool get _isTemplate => _templates.any((t) => t.id == widget.pageId);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_feed == null) {
      _feed = context.app.feed..addListener(_onFeed);
      _load();
    }
  }

  @override
  void dispose() {
    _feed?.removeListener(_onFeed);
    super.dispose();
  }

  void _onFeed(ChangeBatch b) {
    if (b.treeChanged || b.touches(widget.parent)) _load();
  }

  Future<void> _load() async {
    final vault = context.app.vault;
    final p = widget.parent;
    try {
      final r = await vault.run(
        (v) async {
          final parent = await v.loadPage(id: p);
          return (
            await v.listTemplates(parent: p),
            await v.defaultTemplate(collection: p),
            parent.title,
            firstLineOfTexts(parent.blocks.map((b) => b.text)),
          );
        },
      );
      if (!mounted) return;
      setState(() {
        _templates = r.$1;
        _default = r.$2;
        _parentTitle = r.$3;
        _parentLine = r.$4;
      });
    } catch (_) {
      // The parent went away; the page closes itself.
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_isTemplate) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [_pad(_banner(context)), widget.child],
      );
    }
    return widget.child;
  }

  Widget _pad(Widget w) => Padding(
    padding: EdgeInsets.only(left: widget.left, bottom: 8),
    child: Align(
      alignment: Alignment.topLeft,
      child: SizedBox(width: widget.width, child: w),
    ),
  );

  /// "Template in Parent" with its actions, at the top of a template.
  Widget _banner(BuildContext context) {
    final c = context.colors;
    final isDefault = _default == widget.pageId;
    final vault = context.app.vault;
    Widget action(String key, String label, VoidCallback onTap) => Hover(
      key: ValueKey(key),
      onTap: onTap,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
      child: Text(
        label,
        style: ts(context.type.uiSmall, c.textSecondary, weight: 500),
      ),
    );
    return Container(
      key: const ValueKey('template-banner'),
      margin: const EdgeInsets.only(top: 4),
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: BorderRadius.circular(Radii.md),
      ),
      child: Row(
        children: [
          DbIcon(DbIco.template, size: 16, color: c.iconSecondary),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              isDefault
                  ? 'Default template for new pages in ${displayTitle(_parentTitle, firstLine: _parentLine)}'
                  : 'Template for new pages in ${displayTitle(_parentTitle, firstLine: _parentLine)}',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textSecondary),
            ),
          ),
          if (!isDefault)
            action(
              'template-make-default',
              'Set as default',
              () => vault.run(
                (v) => v.setDefaultTemplate(
                  collection: widget.parent,
                  template: widget.pageId,
                ),
                writes: true,
              ),
            ),
          action(
            'template-unmark',
            'Turn into page',
            () => vault.run(
              (v) => v.setTemplate(page: widget.pageId, isTemplate: false),
              writes: true,
            ),
          ),
        ],
      ),
    );
  }
}

/// The first body line of page [id], or empty when it cannot be loaded.
Future<String> _firstLineOf(NotesVault v, String id) async {
  try {
    return firstLineOfTexts((await v.loadPage(id: id)).blocks.map((b) => b.text));
  } catch (_) {
    return '';
  }
}

/// The first body line of each untitled page in [rows], by id (templates
/// are often untitled, and show their first line in the menus).
Future<Map<String, String>> _firstLinesOf(NotesVault v, List<PageRow> rows) async => {
      for (final r in rows)
        if (r.title.isEmpty) r.id: await _firstLineOf(v, r.id),
    };

/// The one empty state of an empty page, under its title: "Blank" (focuses
/// the body), one chip per template
/// of its parent, and "+ Template" (a new template for the parent).
class PageStarters extends StatefulWidget {
  const PageStarters({
    super.key,
    required this.pageId,
    required this.parent,
    required this.left,
    required this.width,
    required this.onBlank,
  });
  final String pageId;
  final String? parent;
  final double left, width;
  final VoidCallback onBlank;

  @override
  State<PageStarters> createState() => _PageStartersState();
}

class _PageStartersState extends State<PageStarters> {
  List<PageRow> _templates = const [];
  String? _default;
  Map<String, String> _lines = const {};
  ChangeFeed? _feed;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_feed == null && widget.parent != null) {
      _feed = context.app.feed..addListener(_onFeed);
      _load();
    }
  }

  @override
  void dispose() {
    _feed?.removeListener(_onFeed);
    super.dispose();
  }

  /// A template's chip name: its title, else its first body line.
  String _nameOf(PageRow t) => displayTitle(t.title, firstLine: _lines[t.id] ?? '');

  void _onFeed(ChangeBatch b) {
    if (b.treeChanged || b.touches(widget.parent!)) _load();
  }

  Future<void> _load() async {
    final p = widget.parent;
    if (p == null) return;
    try {
      final r = await context.app.vault.run(
        (v) async {
          final rows = await v.listTemplates(parent: p);
          return (rows, await v.defaultTemplate(collection: p), await _firstLinesOf(v, rows));
        },
      );
      if (!mounted) return;
      setState(() {
        _templates = [
          for (final t in r.$1)
            if (t.id != widget.pageId) t,
        ];
        _default = r.$2;
        _lines = r.$3;
      });
    } catch (_) {
      // The parent went away; the page closes itself.
    }
  }

  Future<void> _pick(PageRow t) => context.app.vault.run(
    (v) => v.applyTemplate(page: widget.pageId, template: t.id),
    writes: true,
  );

  Future<void> _newTemplate() async {
    final id = await context.app.vault.run(
      (v) => v.createTemplate(parent: widget.parent!, title: ''),
      writes: true,
    );
    if (mounted) await nav.openPage(context, id, '', isNew: true, stack: true);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget chip(
      String key,
      String label,
      DbIco? icon,
      VoidCallback onTap, {
      bool active = false,
    }) => ChipBtn(
      key: ValueKey(key),
      label: label,
      icon: icon,
      active: active,
      height: 26,
      radius: Radii.md,
      onTap: onTap,
    );
    return Padding(
      padding: EdgeInsets.only(left: widget.left, top: 2, bottom: 10),
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: widget.width,
          child: Wrap(
            key: const ValueKey('page-starters'),
            spacing: 6,
            runSpacing: 6,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              chip('start-blank', 'Blank', DbIco.text, widget.onBlank),
              for (final t in _templates)
                chip(
                  'template-pick-${_nameOf(t)}',
                  _nameOf(t),
                  DbIco.template,
                  () => _pick(t),
                  active: t.id == _default,
                ),
              if (widget.parent != null)
                Semantics(
                  button: true,
                  label: 'New template',
                  excludeSemantics: true,
                  child: Hover(
                    key: const ValueKey('template-add'),
                    onTap: _newTemplate,
                    radius: Radii.md,
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    child: SizedBox(
                      height: 26,
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon20(Ico.plus, size: 14, color: c.iconTertiary),
                          const SizedBox(width: 4),
                          Text(
                            'Template',
                            style: ts(
                              context.type.uiSmall,
                              c.textTertiary,
                              weight: 500,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The "New ▾" menu of a database (or any page's new children): its
/// templates (the default marked), "Empty page", "Clear default template"
/// and "New template". [create] makes a child from a template (null: empty)
/// and opens it; [open] opens a template to edit it; [setDefault] changes
/// the default.
Future<void> showTemplateMenu(
  BuildContext anchor, {
  required String parent,
  required String title,
  required String? defaultId,
  required Future<void> Function(String? template) create,
  required void Function(String id) open,
  required Future<void> Function(String? id) setDefault,
}) async {
  final vault = anchor.app.vault;
  final r = await vault.run((v) async {
    final rows = await v.listTemplates(parent: parent);
    return (rows, title.isEmpty ? await _firstLineOf(v, parent) : '', await _firstLinesOf(v, rows));
  });
  final templates = r.$1;
  final parentLine = r.$2;
  final lines = r.$3;
  if (!anchor.mounted) return;
  return showPanel<void>(anchor, globalRectOf(anchor), (ctx) {
    void close() => Navigator.of(ctx).pop();
    Widget item(PageRow t) {
      final name = displayTitle(t.title, firstLine: lines[t.id] ?? '');
      final isDefault = t.id == defaultId;
      return dbItem(
        ctx,
        name,
        DbIco.template,
        key: ValueKey(isDefault ? 'template-default' : 'template-item-$name'),
        trailing: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (isDefault)
              Text(
                'DEFAULT',
                style: tsRaw(10, 12, 600, ctx.colors.textTertiary),
              ),
            if (!isDefault)
              DbIconBtn(
                DbIco.pin,
                key: ValueKey('template-set-default-$name'),
                size: 22,
                glyph: 14,
                label: 'Set as default',
                onTap: () {
                  close();
                  setDefault(t.id);
                },
              ),
            const SizedBox(width: 4),
            DbIconBtn(
              DbIco.edit,
              key: ValueKey(
                isDefault ? 'template-edit' : 'template-edit-$name',
              ),
              size: 22,
              glyph: 14,
              label: 'Edit template',
              onTap: () {
                close();
                open(t.id);
              },
            ),
          ],
        ),
        onTap: () {
          close();
          create(t.id);
        },
      );
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        MenuHeader('Templates for ${displayTitle(title, firstLine: parentLine)}'),
        for (final t in templates) item(t),
        dbItem(
          ctx,
          'Empty page',
          null,
          ico: Ico.page,
          key: const ValueKey('template-empty'),
          onTap: () {
            close();
            create(null);
          },
        ),
        if (defaultId != null)
          dbItem(
            ctx,
            'Clear default template',
            null,
            ico: Ico.close,
            key: const ValueKey('template-clear'),
            onTap: () {
              close();
              setDefault(null);
            },
          ),
        const MenuSeparator(),
        dbItem(
          ctx,
          'New template',
          null,
          ico: Ico.plus,
          key: const ValueKey('template-new'),
          onTap: () async {
            close();
            final id = await vault.run(
              (v) => v.createTemplate(parent: parent, title: ''),
              writes: true,
            );
            // The first template becomes the default, as before templates
            // could be many.
            if (defaultId == null && templates.isEmpty) await setDefault(id);
            open(id);
          },
        ),
      ],
    );
  });
}
