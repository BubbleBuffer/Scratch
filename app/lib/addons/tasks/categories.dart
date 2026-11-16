import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../core/vault.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';

Future<void> showCalendarCategories(BuildContext context) =>
    showPickerSheet<void>(
      context,
      (_) =>
          CalendarCategoriesPanel(vault: const AddonServices().vault(context)),
      desktopWidth: 410,
      desktopHeight: 640,
      shrink: false,
      sheetHeightFactor: .82,
    );

class CalendarCategoriesPanel extends StatefulWidget {
  const CalendarCategoriesPanel({
    super.key,
    required this.vault,
    this.onChanged,
    this.compact = false,
  });
  final Vault vault;
  final VoidCallback? onChanged;
  final bool compact;
  @override
  State<CalendarCategoriesPanel> createState() =>
      _CalendarCategoriesPanelState();
}

class _CalendarCategoriesPanelState extends State<CalendarCategoriesPanel> {
  CalendarSettings? _settings;
  List<PageRow> _databases = [];
  bool _busy = false;
  String? _error;
  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final settings = await widget.vault.listCalendars();
      final databases = <PageRow>[];
      // Enumerate the vault so an overlay with no items in this period is still
      // available to turn back on. Pages inside databases are not calendars.
      for (var offset = 0; ; offset += 500) {
        final pages = await widget.vault.listPages(offset: offset, limit: 500);
        final dated = await Future.wait([
          for (final page in pages.where((p) => p.isDatabase))
            () async {
              final schema = await widget.vault.run(
                (v) => v.getSchema(collection: page.id),
              );
              return schema.any(
                    (p) =>
                        p.kind == PropKind.date ||
                        p.kind == PropKind.createdTime ||
                        p.kind == PropKind.updatedTime,
                  )
                  ? page
                  : null;
            }(),
        ]);
        databases.addAll(dated.whereType<PageRow>());
        if (pages.length < 500) break;
      }
      databases.sort((a, b) => a.title.compareTo(b.title));
      if (!mounted) return;
      setState(() {
        _settings = settings;
        _databases = databases;
      });
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not load calendars. Try again.');
      }
    }
  }

  Future<void> _write(Future<void> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await action();
      await _load();
      widget.onChanged?.call();
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not save the calendar. Try again.');
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  CalendarCategory _copy(CalendarCategory c, {bool? calendar, bool? today}) =>
      CalendarCategory(
        id: c.id,
        name: c.name,
        color: c.color,
        showInCalendar: calendar ?? c.showInCalendar,
        showInToday: today ?? c.showInToday,
      );

  Future<void> _edit([CalendarCategory? calendar]) async {
    final result = await showPickerSheet<CalendarCategory>(
      context,
      (_) => _CalendarEditor(calendar: calendar),
      desktopWidth: 350,
      desktopHeight: 360,
    );
    if (result == null || !mounted) return;
    await _write(() async {
      if (calendar == null) {
        await widget.vault.createCalendar(
          name: result.name,
          color: result.color,
          showInCalendar: result.showInCalendar,
          showInToday: result.showInToday,
        );
      } else {
        await widget.vault.updateCalendar(result);
      }
    });
  }

  Future<void> _menu(BuildContext anchor, CalendarCategory calendar) async {
    final action = await showPopover<String>(
      anchor,
      globalRectOf(anchor),
      (c) => MenuList(
        children: [
          MenuItem(
            label: 'Rename or change colour',
            icon: Ico.compose,
            onTap: () => Navigator.of(c).pop('edit'),
          ),
          MenuItem(
            label: 'Delete calendar',
            icon: Ico.trash,
            danger: true,
            onTap: () => Navigator.of(c).pop('delete'),
          ),
        ],
      ),
    );
    if (!mounted) return;
    if (action == 'edit') {
      await _edit(calendar);
      return;
    }
    if (action == 'delete') {
      final confirmed = await showConfirm(
        context,
        title: 'Delete ${calendar.name}?',
        message: 'Tasks keep their dates.',
        confirmLabel: 'Delete calendar',
        danger: true,
      );
      if (confirmed && mounted) {
        await _write(() => widget.vault.deleteCalendar(calendar.id));
      }
    }
  }

  Widget _toggle(String label, bool value, VoidCallback action) => Semantics(
    label: label,
    toggled: value,
    child: Hover(
      onTap: _busy ? null : action,
      child: Padding(
        padding: const EdgeInsets.all(Space.xs),
        child: Toggle(value: value, onChanged: null),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final settings = _settings;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(Space.lg, Space.lg, Space.sm, Space.md),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  'Calendars',
                  style: tsDisplay(context.type.h3, colors.textPrimary),
                ),
              ),
              if (!widget.compact)
                IconBtn(
                  Ico.close,
                  semanticLabel: 'Close calendars',
                  onTap: () => Navigator.of(context).pop(),
                ),
            ],
          ),
        ),
        if (_error != null)
          Padding(
            padding: const EdgeInsets.all(Space.md),
            child: Text(
              _error!,
              style: ts(context.type.uiSmall, colors.inlineCodeText),
            ),
          ),
        if (settings == null)
          Padding(
            padding: const EdgeInsets.all(Space.lg),
            child: Text(
              'Loading calendars…',
              style: ts(context.type.ui, colors.textSecondary),
            ),
          )
        else
          Expanded(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(Space.md, 0, Space.md, Space.xl),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      const Spacer(),
                      Text(
                        'Calendar',
                        style: ts(context.type.micro, colors.textTertiary, weight: Weight.medium),
                      ),
                      const Gap(Space.sm),
                      Text(
                        'Today',
                        style: ts(context.type.micro, colors.textTertiary, weight: Weight.medium),
                      ),
                      const Gap(Space.xxl),
                    ],
                  ),
                  _heading(context, 'My calendars'),
                  for (final c in settings.calendars)
                    SizedBox(
                      height: 44,
                      child: Row(
                        children: [
                          SpaceDot(c.color),
                          const Gap(Space.sm),
                          Expanded(
                            child: Text(
                              c.name,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: ts(context.type.ui, colors.textPrimary),
                            ),
                          ),
                          _toggle(
                            'Show ${c.name} in calendar',
                            c.showInCalendar,
                            () => _write(
                              () => widget.vault.updateCalendar(
                                _copy(c, calendar: !c.showInCalendar),
                              ),
                            ),
                          ),
                          _toggle(
                            'Show ${c.name} in Today',
                            c.showInToday,
                            () => _write(
                              () => widget.vault.updateCalendar(
                                _copy(c, today: !c.showInToday),
                              ),
                            ),
                          ),
                          Builder(
                            builder: (b) => IconBtn(
                              Ico.more,
                              semanticLabel: '${c.name} calendar menu',
                              onTap: _busy ? null : () => _menu(b, c),
                            ),
                          ),
                        ],
                      ),
                    ),
                  _heading(context, 'From databases'),
                  if (_databases.isEmpty)
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: Space.md),
                      child: Text(
                        'No databases yet.',
                        style: ts(context.type.uiSmall, colors.textSecondary),
                      ),
                    ),
                  for (final db in _databases)
                    SizedBox(
                      height: 44,
                      child: Row(
                        children: [
                          PageIcon(
                            icon: db.icon,
                            size: IconSize.sm.px,
                            color: colors.iconSecondary,
                          ),
                          const Gap(Space.sm),
                          Expanded(
                            child: Text(
                              displayTitle(db.title),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: ts(context.type.ui, colors.textPrimary),
                            ),
                          ),
                          _toggle(
                            'Show ${displayTitle(db.title)} in calendar',
                            !settings.databases.any(
                              (d) => d.id == db.id && !d.showInCalendar,
                            ),
                            () => _write(
                              () => widget.vault.setDatabaseCalendarVisibility(
                                db.id,
                                showInCalendar: settings.databases.any(
                                  (d) => d.id == db.id && !d.showInCalendar,
                                ),
                              ),
                            ),
                          ),
                          const Gap(Space.xxxl),
                        ],
                      ),
                    ),
                  const Gap(Space.md),
                  TextBtn(
                    'New calendar',
                    onTap: _busy ? null : () => _edit(),
                    size: ControlSize.xl,
                  ),
                  const Gap(Space.lg),
                  Text(
                    'Calendar and Today visibility are independent. Hiding a database keeps every row and its dates.',
                    style: ts(context.type.uiSmall, colors.textSecondary),
                  ),
                ],
              ),
            ),
          ),
      ],
    );
  }

  Widget _heading(BuildContext context, String title) => Padding(
    padding: const EdgeInsets.only(top: Space.xl, bottom: Space.sm),
    child: Text(
      title,
      style: ts(
        context.type.uiSmall,
        context.colors.textSecondary,
        weight: Weight.bold,
      ),
    ),
  );
}

class _CalendarEditor extends StatefulWidget {
  const _CalendarEditor({this.calendar});
  final CalendarCategory? calendar;
  @override
  State<_CalendarEditor> createState() => _CalendarEditorState();
}

class _CalendarEditorState extends State<_CalendarEditor> {
  late final TextEditingController _name = TextEditingController(
    text: widget.calendar?.name ?? '',
  );
  final _focus = FocusNode();
  late String _color = widget.calendar?.color ?? 'apricot';
  @override
  void dispose() {
    _name.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _save() {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    Navigator.of(context).pop(
      CalendarCategory(
        id: widget.calendar?.id ?? '',
        name: name,
        color: _color,
        showInCalendar: widget.calendar?.showInCalendar ?? true,
        showInToday: widget.calendar?.showInToday ?? true,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.all(Space.xl),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          widget.calendar == null ? 'New calendar' : 'Edit calendar',
          style: tsDisplay(context.type.h3, context.colors.textPrimary),
        ),
        const Gap(Space.lg),
        FieldFrame(
          child: PlainField(
            controller: _name,
            focusNode: _focus,
            style: ts(context.type.body, context.colors.textPrimary),
            placeholder: 'Calendar name',
            autofocus: true,
            onChanged: (_) => setState(() {}),
            onSubmitted: (_) => _save(),
          ),
        ),
        const Gap(Space.lg),
        Wrap(
          spacing: Space.md,
          runSpacing: Space.sm,
          children: [
            for (final color in spaceColorNames)
              Semantics(
                label: color,
                selected: _color == color,
                child: Hover(
                  onTap: () => setState(() => _color = color),
                  selected: _color == color,
                  child: SizedBox(
                    width: 36,
                    height: 36,
                    child: Center(
                      child: Container(
                        width: 26,
                        height: 26,
                        decoration: BoxDecoration(
                          color: spaceColor(
                            color,
                            context.colors.dark
                                ? Brightness.dark
                                : Brightness.light,
                          ),
                          shape: BoxShape.circle,
                          border: _color == color
                              ? Border.all(
                                  color: context.colors.textPrimary,
                                  width: Stroke.bold,
                                )
                              : null,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
        const Gap(Space.xl),
        SecondaryButton(
          label: 'Save calendar',
          onTap: _name.text.trim().isEmpty ? null : _save,
        ),
      ],
    ),
  );
}
