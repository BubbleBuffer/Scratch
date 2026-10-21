import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/note_drag_payload.dart';
import '../../services/addon_services.dart';
import '../../state/agenda_store.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'calendar_model.dart';
import 'calendar_reschedule.dart';
import 'calendar_note_drop.dart';
import 'calendar_drop_toast.dart';
import 'categories.dart';
import 'navigation.dart';
import 'item_card.dart';

const calendarRouteName = '/calendar';

Future<void> openCalendar(BuildContext context, {DateTime? day}) async {
  if (AppScope.maybeOf(context) == null) return;
  if (day != null) context.app.agenda.selectDay(day);
  final stack = PageStackScope.of(context);
  if (stack.currentRoute?.settings.name == calendarRouteName) return;
  final navigator = stack.navigator ?? Navigator.of(context);
  navigator.popUntil((r) => r.isFirst);
  await navigator.push(
    pageRoute(
      (_) => const VaultCalendarScreen(),
      motion: Motion.of(context),
      mobile: context.mobile,
      settings: const RouteSettings(name: calendarRouteName),
    ),
  );
}

enum VaultCalendarPeriod { month, week, day }

class VaultCalendarScreen extends StatefulWidget {
  const VaultCalendarScreen({super.key});
  @override
  State<VaultCalendarScreen> createState() => _VaultCalendarScreenState();
}

class _VaultCalendarScreenState extends State<VaultCalendarScreen> {
  VaultCalendarStore? _store;
  late DateTime _selected;
  late DateTime _headingMonth;
  VaultCalendarPeriod _period = VaultCalendarPeriod.month;
  bool _categories = true;
  String? _error;
  final _focus = FocusNode();
  final _monthKey = GlobalKey<_ContinuousMonthsState>();
  Timer? _clock;
  bool _dialogOpen = false;
  VoidCallback? _dropToast;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_store != null) return;
    _selected = context.app.agenda.selectedDay;
    _headingMonth = calendarMonth(_selected);
    _store = VaultCalendarStore(context.app.vault, context.app.feed)
      ..addListener(_changed);
    _loadPeriod();
    _clock = Timer.periodic(const Duration(minutes: 1), (_) {
      if (mounted) setState(() {});
    });
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    _dropToast?.call();
    _clock?.cancel();
    _store?.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _loadPeriod() {
    if (_period == VaultCalendarPeriod.month) {
      final first = calendarMonday(calendarMonth(_selected, -2));
      unawaited(
        _store!.loadRange(
          first,
          calendarAddDays(calendarMonday(calendarMonth(_selected, 3)), 7),
        ),
      );
    } else {
      final first = _period == VaultCalendarPeriod.week
          ? calendarMonday(_selected)
          : calendarDay(_selected);
      unawaited(
        _store!.loadRange(
          first,
          calendarAddDays(first, _period == VaultCalendarPeriod.week ? 7 : 1),
        ),
      );
    }
  }

  void _select(DateTime day, {bool scroll = false}) {
    setState(() {
      _selected = calendarDay(day);
      _headingMonth = calendarMonth(day);
    });
    context.app.agenda.selectDay(day);
    if (_period == VaultCalendarPeriod.month) {
      if (scroll) _monthKey.currentState?.showDay(day);
    } else {
      _loadPeriod();
    }
  }

  void _view(VaultCalendarPeriod period) {
    setState(() => _period = period);
    _loadPeriod();
  }

  void _move(int direction) {
    final target = switch (_period) {
      VaultCalendarPeriod.month => DateTime(
        _headingMonth.year,
        _headingMonth.month + direction,
        math.min(
          _selected.day,
          DateTime(
            _headingMonth.year,
            _headingMonth.month + direction + 1,
            0,
          ).day,
        ),
      ),
      VaultCalendarPeriod.week => calendarAddDays(_selected, direction * 7),
      VaultCalendarPeriod.day => calendarAddDays(_selected, direction),
    };
    _select(target, scroll: true);
  }

  Future<void> _create(DateTime date, {int? hour}) async {
    if (_dialogOpen) return;
    _dialogOpen = true;
    try {
      await showTaskComposer(context, date: date, hour: hour);
    } finally {
      _dialogOpen = false;
      if (mounted) {
        _focus.requestFocus();
        unawaited(_store!.reload());
      }
    }
  }

  Future<void> _jump() async {
    if (_dialogOpen) return;
    _dialogOpen = true;
    final date = await showPickerSheet<DateTime>(
      context,
      (_) => CalendarJumpPicker(selected: _selected),
      desktopWidth: 350,
      desktopHeight: 440,
    );
    _dialogOpen = false;
    if (!mounted) return;
    if (date != null) _select(date, scroll: true);
    _focus.requestFocus();
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is! KeyDownEvent ||
        _dialogOpen ||
        ModalRoute.of(context)?.isCurrent == false) {
      return KeyEventResult.ignored;
    }
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused?.widget is EditableText ||
        focused?.findAncestorWidgetOfExactType<EditableText>() != null) {
      return KeyEventResult.ignored;
    }
    final keyboard = HardwareKeyboard.instance;
    if (keyboard.isControlPressed ||
        keyboard.isAltPressed ||
        keyboard.isMetaPressed) {
      return KeyEventResult.ignored;
    }
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.keyT) {
      _select(clockNow(), scroll: true);
    } else if (key == LogicalKeyboardKey.keyM) {
      _view(VaultCalendarPeriod.month);
    } else if (key == LogicalKeyboardKey.keyW) {
      _view(VaultCalendarPeriod.week);
    } else if (key == LogicalKeyboardKey.keyD) {
      _view(VaultCalendarPeriod.day);
    } else if (key == LogicalKeyboardKey.arrowLeft ||
        key == LogicalKeyboardKey.keyK) {
      _move(-1);
    } else if (key == LogicalKeyboardKey.arrowRight ||
        key == LogicalKeyboardKey.keyJ) {
      _move(1);
    } else if (key == LogicalKeyboardKey.arrowUp) {
      _select(calendarAddDays(_selected, -1), scroll: true);
    } else if (key == LogicalKeyboardKey.arrowDown) {
      _select(calendarAddDays(_selected, 1), scroll: true);
    } else if (key == LogicalKeyboardKey.keyG) {
      unawaited(_jump());
    } else if (key == LogicalKeyboardKey.keyN ||
        key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter) {
      unawaited(_create(_selected));
    } else if (key == LogicalKeyboardKey.escape) {
      Navigator.of(context).maybePop();
    } else if (event.character == '?') {
      _shortcuts();
    } else {
      return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  void _shortcuts() => showPickerSheet<void>(
    context,
    (ctx) => Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Keyboard shortcuts',
            style: tsDisplay(ctx.type.h3, ctx.colors.textPrimary),
          ),
          const SizedBox(height: 18),
          for (final pair in const [
            ('T', 'Back to today'),
            ('M / W / D', 'Month / week / day'),
            ('← / → · K / J', 'Previous / next period'),
            ('↑ / ↓', 'Previous / next day'),
            ('G', 'Jump to date'),
            ('N / Enter', 'New task on selected day'),
            ('Esc', 'Close'),
            ('?', 'Keyboard shortcuts'),
          ])
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 7),
              child: Row(
                children: [
                  SizedBox(
                    width: 100,
                    child: Text(
                      pair.$1,
                      style: ts(
                        ctx.type.uiSmall,
                        ctx.colors.textSecondary,
                        mono: true,
                      ),
                    ),
                  ),
                  Expanded(
                    child: Text(
                      pair.$2,
                      style: ts(ctx.type.ui, ctx.colors.textPrimary),
                    ),
                  ),
                ],
              ),
            ),
        ],
      ),
    ),
    desktopWidth: 390,
    desktopHeight: 460,
  );

  Future<void> _open(AgendaItem item) async {
    if (_standaloneTask(item)) {
      await openTask(context, item.page);
    } else {
      unawaited(
        openPage(context, item.page, item.title, initialBlock: item.block),
      );
    }
  }

  Future<void> _toggle(AgendaItem item) async {
    try {
      await context.app.vault.run(
        (v) => v.setAgendaDone(
          page: item.page,
          block: item.block,
          done: !item.done,
        ),
        writes: true,
      );
      await _store!.reload();
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not update the task. Try again.');
      }
    }
  }

  Future<void> _reschedule(Object item, DateTime day, {int? hour}) async {
    try {
      if (item is NoteDragPayload) {
        final result = await dropNoteOnCalendar(
          context.app.vault,
          item,
          day,
          hour: hour,
        );
        if (!mounted) return;
        _dropToast?.call();
        _dropToast = showCalendarDropToast(
          context,
          result.duplicate
              ? 'Already scheduled on this day.'
              : 'Added to calendar.',
          undo: result.undo == null
              ? null
              : () async {
                  await result.undo!();
                  if (mounted) await _store!.reload();
                },
          onError: (_) {
            if (mounted) {
              setState(
                () => _error = 'Could not undo the calendar change. The note may have changed.',
              );
            }
          },
        );
      } else if (item is AgendaItem) {
        await rescheduleCalendarItem(context.app.vault, item, day, hour: hour);
      } else {
        return;
      }
      if (!mounted) return;
      _select(day);
      await _store!.reload();
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not move the item. Try again.');
      }
    }
  }

  Widget _card(AgendaItem item) => ShortItemCard(
    item: item,
    calendars: _store!.settings,
    sourceTitle: _store!.sourceTitles[AgendaStore.sourceId(item)],
    checkbox: _store!.checkboxes[item.page] ?? true,
    onOpen: () => _open(item),
    onToggle: () => _toggle(item),
  );

  Future<void> _dayPopup(BuildContext anchor, DateTime day) async {
    _select(day);
    await showPickerSheet<void>(
      context,
      (ctx) => AnimatedBuilder(
        animation: _store!,
        builder: (ctx, _) => Padding(
          padding: const EdgeInsets.all(18),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      calendarShortDate(day),
                      style: tsDisplay(ctx.type.h3, ctx.colors.textPrimary),
                    ),
                  ),
                  IconBtn(
                    Ico.close,
                    semanticLabel: 'Close day',
                    onTap: () => Navigator.of(ctx).pop(),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              ConstrainedBox(
                constraints: const BoxConstraints(maxHeight: 360),
                child: SingleChildScrollView(
                  child: Column(
                    children: [
                      for (final item in _store!.onDay(day)) _card(item),
                      if (_store!.onDay(day).isEmpty) _empty(ctx),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 14),
              SecondaryButton(
                label: 'New task',
                onTap: () {
                  Navigator.of(ctx).pop();
                  unawaited(_create(day));
                },
              ),
            ],
          ),
        ),
      ),
      desktopWidth: 570,
      desktopHeight: 540,
    );
  }

  Widget _empty(BuildContext ctx) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 22),
    child: Text(
      'Nothing planned',
      style: ts(ctx.type.ui, ctx.colors.textSecondary),
    ),
  );

  @override
  Widget build(BuildContext context) {
    if (_store == null) return const SizedBox.shrink();
    final mobile = context.mobile;
    final compactDesktop = !mobile && MediaQuery.sizeOf(context).width < 1120;
    final c = context.colors;
    final title = _period == VaultCalendarPeriod.day
        ? calendarShortDate(_selected)
        : calendarMonthTitle(_headingMonth);
    final body = Focus(
      autofocus: true,
      focusNode: _focus,
      onKeyEvent: _key,
      child: Container(
        margin: mobile ? EdgeInsets.zero : const EdgeInsets.all(14),
        clipBehavior: Clip.antiAlias,
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: mobile ? BorderRadius.zero : Radii.panel,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 6,
              color: spaceColor(
                'sun',
                c.dark ? Brightness.dark : Brightness.light,
              ),
            ),
            Padding(
              padding: EdgeInsets.fromLTRB(mobile ? 12 : 20, 12, 12, 8),
              child: Row(
                children: [
                  IconBtn(
                    Ico.chevronLeft,
                    size: mobile ? 44 : 28,
                    semanticLabel: 'Back to Pages',
                    onTap: () => Navigator.of(context).maybePop(),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Hover(
                      onTap: _jump,
                      child: Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: tsDisplay(
                          mobile ? context.type.h2 : context.type.h1,
                          c.textPrimary,
                        ),
                      ),
                    ),
                  ),
                  if (!mobile && !compactDesktop) _desktopActions(),
                ],
              ),
            ),
            if (compactDesktop)
              Padding(
                padding: const EdgeInsets.fromLTRB(20, 0, 12, 10),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: _desktopActions(),
                ),
              ),
            if (mobile)
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 0, 10, 8),
                child: Row(
                  children: [
                    _viewButtons(),
                    const Spacer(),
                    IconBtn(
                      Ico.chevronLeft,
                      size: 44,
                      semanticLabel: 'Previous period',
                      onTap: () => _move(-1),
                    ),
                    TextBtn(
                      'Today',
                      height: 44,
                      onTap: () => _select(clockNow(), scroll: true),
                    ),
                    IconBtn(
                      Ico.chevronRight,
                      size: 44,
                      semanticLabel: 'Next period',
                      onTap: () => _move(1),
                    ),
                    IconBtn(
                      Ico.settings,
                      size: 44,
                      semanticLabel: 'Calendars',
                      onTap: () => showCalendarCategories(context),
                    ),
                  ],
                ),
              ),
            if (_error != null || _store!.error != null)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 18),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        _error ?? _store!.error!,
                        style: ts(context.type.uiSmall, c.inlineCodeText),
                      ),
                    ),
                    TextBtn(
                      'Retry',
                      onTap: () {
                        setState(() => _error = null);
                        unawaited(_store!.reload());
                      },
                    ),
                  ],
                ),
              ),
            if (_store!.loading && _store!.items.isEmpty)
              Padding(
                padding: const EdgeInsets.all(8),
                child: Text(
                  'Loading calendar…',
                  style: ts(context.type.uiSmall, c.textSecondary),
                ),
              ),
            Expanded(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (!mobile && _categories)
                    SizedBox(
                      width: 225,
                      child: Container(
                        decoration: BoxDecoration(
                          border: Border(right: BorderSide(color: c.border)),
                        ),
                        child: CalendarCategoriesPanel(
                          vault: context.app.vault,
                          compact: true,
                          onChanged: () => unawaited(_store!.reload()),
                        ),
                      ),
                    ),
                  Expanded(child: _content()),
                ],
              ),
            ),
          ],
        ),
      ),
    );
    return ColoredBox(
      color: c.desk,
      child: SafeArea(
        bottom: !mobile,
        child: mobile
            ? ShellMobileFrame(destinationId: 'calendar', child: body)
            : Row(
                children: [
                  if (PageStackScope.maybeOf(context) != null)
                    const Sidebar(currentDestinationId: 'calendar'),
                  Expanded(child: body),
                ],
              ),
      ),
    );
  }

  Widget _viewButtons() => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      for (final period in VaultCalendarPeriod.values)
        Semantics(
          selected: _period == period,
          button: true,
          child: Hover(
            selected: _period == period,
            onTap: () => _view(period),
            padding: EdgeInsets.symmetric(horizontal: context.mobile ? 7 : 10),
            child: SizedBox(
              height: context.mobile ? 44 : 30,
              child: Center(
                child: Text(
                  period.name[0].toUpperCase() + period.name.substring(1),
                  style: ts(
                    context.type.uiSmall,
                    context.colors.textPrimary,
                    weight: _period == period ? 700 : 500,
                  ),
                ),
              ),
            ),
          ),
        ),
    ],
  );

  Widget _desktopActions() => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      IconBtn(
        Ico.chevronLeft,
        semanticLabel: 'Previous period',
        onTap: () => _move(-1),
      ),
      TextBtn('Today', onTap: () => _select(clockNow(), scroll: true)),
      IconBtn(
        Ico.chevronRight,
        semanticLabel: 'Next period',
        onTap: () => _move(1),
      ),
      const SizedBox(width: 12),
      _viewButtons(),
      const SizedBox(width: 12),
      TextBtn('Jump to date', onTap: _jump),
      IconBtn(
        Ico.settings,
        semanticLabel: 'Calendars',
        selected: _categories,
        onTap: () => setState(() => _categories = !_categories),
      ),
      const SizedBox(width: 8),
      PrimaryButton(
        label: 'New task',
        icon: Ico.plus,
        height: 36,
        onTap: () => _create(_selected),
      ),
    ],
  );

  Widget _content() {
    if (_period == VaultCalendarPeriod.month) {
      final month = _ContinuousMonths(
        key: _monthKey,
        selected: _selected,
        store: _store!,
        onSelected: (anchor, day) {
          if (context.mobile) {
            _select(day);
          } else {
            unawaited(_dayPopup(anchor, day));
          }
        },
        onCreate: (day) => _create(day),
        onOpen: _open,
        onToggle: _toggle,
        onMove: _reschedule,
        onMonth: (day) {
          if (mounted && !sameDay(_headingMonth, day)) {
            setState(() => _headingMonth = day);
          }
        },
      );
      if (!context.mobile) return month;
      return Column(
        children: [
          SizedBox(height: 310, child: month),
          Expanded(child: _phoneAgenda()),
        ],
      );
    }
    if (context.mobile) {
      return GestureDetector(
        onHorizontalDragEnd: (details) {
          if (details.primaryVelocity!.abs() > 100) {
            _move(details.primaryVelocity! < 0 ? 1 : -1);
          }
        },
        child: Column(
          children: [
            if (_period == VaultCalendarPeriod.week) _weekStrip(),
            Expanded(child: _phoneAgenda()),
          ],
        ),
      );
    }
    final days = _period == VaultCalendarPeriod.week
        ? [
            for (var i = 0; i < 7; i++)
              calendarAddDays(calendarMonday(_selected), i),
          ]
        : [_selected];
    final grid = _CalendarTimeGrid(
      days: days,
      store: _store!,
      onCreate: (day, hour) => _create(day, hour: hour),
      onOpen: _open,
      onToggle: _toggle,
      onMove: _reschedule,
      onSelect: (day) => _select(day),
    );
    if (_period == VaultCalendarPeriod.week) return grid;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(child: grid),
        SizedBox(
          width: 270,
          child: Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              border: Border(left: BorderSide(color: context.colors.border)),
            ),
            child: SingleChildScrollView(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'Due this day',
                    style: tsDisplay(
                      context.type.h3,
                      context.colors.textPrimary,
                    ),
                  ),
                  const SizedBox(height: 14),
                  for (final item
                      in _store!
                          .onDay(_selected)
                          .where(
                            (i) => !i.hasTime && i.kind != AgendaKind.mention,
                          ))
                    _card(item),
                  if (!_store!
                      .onDay(_selected)
                      .any((i) => !i.hasTime && i.kind != AgendaKind.mention))
                    _empty(context),
                  const SizedBox(height: 24),
                  Text(
                    'Mentioned in',
                    style: tsDisplay(
                      context.type.h3,
                      context.colors.textPrimary,
                    ),
                  ),
                  const SizedBox(height: 14),
                  for (final item
                      in _store!
                          .onDay(_selected)
                          .where((i) => i.kind == AgendaKind.mention))
                    _card(item),
                  if (!_store!
                      .onDay(_selected)
                      .any((i) => i.kind == AgendaKind.mention))
                    Text(
                      'No date mentions on this day.',
                      style: ts(
                        context.type.uiSmall,
                        context.colors.textSecondary,
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _weekStrip() => Padding(
    padding: const EdgeInsets.all(12),
    child: Row(
      children: [
        for (var n = 0; n < 7; n++)
          Expanded(
            child: Builder(
              builder: (context) {
                final day = calendarAddDays(calendarMonday(_selected), n);
                return Hover(
                  onTap: () => _select(day),
                  selected: sameDay(day, _selected),
                  child: SizedBox(
                    height: 62,
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Text(
                          weekdays[n].substring(0, 3),
                          style: ts(
                            context.type.uiSmall,
                            context.colors.textSecondary,
                          ),
                        ),
                        Text(
                          '${day.day}',
                          style: tsDisplay(
                            context.type.h3,
                            context.colors.textPrimary,
                          ),
                        ),
                        Row(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            for (final item in _store!.onDay(day).take(3))
                              Padding(
                                padding: const EdgeInsets.all(1),
                                child: _CalendarDot(
                                  item: item,
                                  settings: _store!.settings,
                                  size: 4,
                                ),
                              ),
                          ],
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
      ],
    ),
  );

  Widget _phoneAgenda() => SingleChildScrollView(
    padding: const EdgeInsets.fromLTRB(16, 12, 16, 80),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                calendarShortDate(_selected),
                style: tsDisplay(context.type.h3, context.colors.textPrimary),
              ),
            ),
            IconBtn(
              Ico.plus,
              size: 44,
              semanticLabel: 'New task on ${calendarShortDate(_selected)}',
              onTap: () => _create(_selected),
            ),
          ],
        ),
        const SizedBox(height: 8),
        for (final item in _store!.onDay(_selected)) _card(item),
        if (_store!.onDay(_selected).isEmpty) _empty(context),
      ],
    ),
  );
}

bool _canDropCalendarPayload(Object payload) =>
    payload is NoteDragPayload ||
    (payload is AgendaItem && canRescheduleCalendarItem(payload));

typedef _MoveCalendarItem = Future<void> Function(
  Object item,
  DateTime day, {
  int? hour,
});

Color _calendarColor(
  BuildContext context,
  AgendaItem item,
  CalendarSettings settings,
) {
  String? color = item.spaceColor;
  for (final calendar in settings.calendars) {
    if (calendar.id == item.calendar) {
      color = calendar.color;
      break;
    }
  }
  return spaceColor(
        color,
        context.colors.dark ? Brightness.dark : Brightness.light,
      ) ??
      context.colors.threadStrong;
}

bool _standaloneTask(AgendaItem item) =>
    item.kind == AgendaKind.task &&
    item.collection == null &&
    item.block == null;

class _CalendarDot extends StatelessWidget {
  const _CalendarDot({
    required this.item,
    required this.settings,
    this.size = 8,
  });
  final AgendaItem item;
  final CalendarSettings settings;
  final double size;
  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: _calendarColor(context, item, settings),
      borderRadius: BorderRadius.circular(2),
    ),
  );
}

Widget _calendarDrag(BuildContext context, AgendaItem item, Widget child) {
  if (!canRescheduleCalendarItem(item)) return child;
  return Draggable<AgendaItem>(
    data: item,
    feedback: DefaultTextStyle(
      style: ts(context.type.uiSmall, context.colors.textPrimary),
      child: Container(
        width: 170,
        padding: const EdgeInsets.all(8),
        decoration: BoxDecoration(
          color: context.colors.bgPrimary,
          borderRadius: Radii.ctl,
          boxShadow: context.colors.seam,
        ),
        child: Text(item.title, maxLines: 1, overflow: TextOverflow.ellipsis),
      ),
    ),
    childWhenDragging: Opacity(opacity: .35, child: child),
    child: child,
  );
}

class _CalendarChip extends StatelessWidget {
  const _CalendarChip({
    required this.item,
    required this.settings,
    required this.onOpen,
    required this.onToggle,
    this.checkbox = true,
  });
  final AgendaItem item;
  final CalendarSettings settings;
  final VoidCallback onOpen, onToggle;
  final bool checkbox;
  @override
  Widget build(BuildContext context) {
    final color = _calendarColor(context, item, settings);
    final start = DateTime.fromMillisecondsSinceEpoch(item.startMs.toInt());
    final child = Semantics(
      button: true,
      label: item.title,
      child: Hover(
        onTap: onOpen,
        background: item.kind == AgendaKind.mention
            ? null
            : color.withValues(alpha: .18),
        borderRadius: BorderRadius.circular(12),
        border: item.kind == AgendaKind.mention
            ? Border.all(color: context.colors.threadStrong)
            : null,
        padding: const EdgeInsets.symmetric(horizontal: 5),
        child: SizedBox(
          height: 22,
          child: Row(
            children: [
              if (item.kind == AgendaKind.task && checkbox)
                StacksCheckbox(
                  checked: item.done,
                  size: 12,
                  onChanged: (_) => onToggle(),
                )
              else
                Icon20(
                  item.kind == AgendaKind.mention ? Ico.page : Ico.clock,
                  size: 12,
                  color: context.colors.iconSecondary,
                ),
              const SizedBox(width: 4),
              if (item.hasTime) ...[
                Text(
                  calendarTime(start.hour * 60 + start.minute),
                  style: tsRaw(10, 14, 400, context.colors.textSecondary),
                ),
                const SizedBox(width: 4),
              ],
              Expanded(
                child: Text(
                  item.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style:
                      tsRaw(
                        12,
                        16,
                        500,
                        item.done
                            ? context.colors.textTertiary
                            : context.colors.textPrimary,
                      ).copyWith(
                        decoration: item.done
                            ? TextDecoration.lineThrough
                            : null,
                      ),
                ),
              ),
              if (item.assignee == 'agent')
                Text(
                  'Agent',
                  semanticsLabel: 'Assigned to Agent',
                  style: tsRaw(10, 14, 700, context.colors.textSecondary),
                ),
            ],
          ),
        ),
      ),
    );
    return _calendarDrag(context, item, child);
  }
}

class _ContinuousMonths extends StatefulWidget {
  const _ContinuousMonths({
    super.key,
    required this.selected,
    required this.store,
    required this.onSelected,
    required this.onCreate,
    required this.onOpen,
    required this.onToggle,
    required this.onMove,
    required this.onMonth,
  });
  final DateTime selected;
  final VaultCalendarStore store;
  final void Function(BuildContext anchor, DateTime day) onSelected;
  final ValueChanged<DateTime> onCreate, onMonth;
  final ValueChanged<AgendaItem> onOpen, onToggle;
  final _MoveCalendarItem onMove;
  @override
  State<_ContinuousMonths> createState() => _ContinuousMonthsState();
}

class _ContinuousMonthsState extends State<_ContinuousMonths> {
  final _scroll = ScrollController();
  late DateTime _first = calendarMonth(widget.selected, -2);
  int _count = 5;
  bool _ready = false, _extending = false;
  double _cellHeight = 124;
  int _weeks(DateTime month) => calendarMonthWeeks(month);
  double _height(DateTime month) => 28 + _weeks(month) * _cellHeight;
  double _offset(DateTime month) {
    var result = 0.0;
    for (
      var n = 0;
      n < _count && calendarMonth(_first, n).isBefore(month);
      n++
    ) {
      result += _height(calendarMonth(_first, n));
    }
    return result;
  }

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_scrolled);
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  void _load() => unawaited(
    widget.store.loadRange(
      calendarMonday(_first),
      calendarAddDays(calendarMonday(calendarMonth(_first, _count)), 7),
    ),
  );
  void showDay(DateTime day) {
    final month = calendarMonth(day);
    if (month.isBefore(_first) ||
        !month.isBefore(calendarMonth(_first, _count))) {
      setState(() {
        _first = calendarMonth(day, -2);
        _count = 5;
      });
      _load();
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      final target = _offset(month)
          .clamp(0.0, _scroll.position.maxScrollExtent);
      if (MediaQuery.disableAnimationsOf(context)) {
        _scroll.jumpTo(target);
      } else {
        unawaited(
          _scroll.animateTo(
            target,
            duration: const Duration(milliseconds: 220),
            curve: Motion.settle,
          ),
        );
      }
    });
  }

  void _scrolled() {
    if (!_ready || _extending || !_scroll.hasClients) return;
    var offset = 0.0;
    for (var n = 0; n < _count; n++) {
      final month = calendarMonth(_first, n);
      final end = offset + _height(month);
      if (_scroll.offset < end) {
        widget.onMonth(month);
        break;
      }
      offset = end;
    }
    if (_scroll.position.extentAfter < 360) {
      setState(() => _count += 2);
      _load();
    } else if (_scroll.offset < 80) {
      _extending = true;
      final nextFirst = calendarMonth(_first, -2);
      final addedHeight =
          _height(nextFirst) + _height(calendarMonth(nextFirst, 1));
      final oldOffset = _scroll.offset;
      setState(() {
        _first = nextFirst;
        _count += 2;
      });
      _load();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _scroll.hasClients) {
          _scroll.jumpTo(oldOffset + addedHeight);
        }
        _extending = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    _cellHeight = context.mobile ? 60 : 124;
    if (!_ready) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !_scroll.hasClients) return;
        _scroll.jumpTo(
          _offset(calendarMonth(widget.selected))
              .clamp(0.0, _scroll.position.maxScrollExtent),
        );
        _ready = true;
      });
    }
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: context.mobile ? 14 : 16),
      child: Column(
        children: [
          SizedBox(
            height: 29,
            child: Row(
              children: [
                for (final name in weekdays)
                  Expanded(
                    child: Center(
                      child: Text(
                        name.substring(0, 3),
                        style: ts(
                          context.type.uiSmall,
                          context.colors.textTertiary,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
          Expanded(
            child: CustomScrollView(
              key: const ValueKey('calendar-month-scroll'),
              controller: _scroll,
              slivers: [
                for (var n = 0; n < _count; n++)
                  ..._monthSlivers(context, calendarMonth(_first, n)),
                const SliverToBoxAdapter(child: SizedBox(height: 40)),
              ],
            ),
          ),
        ],
      ),
    );
  }

  List<Widget> _monthSlivers(BuildContext context, DateTime month) => [
    SliverPersistentHeader(
      pinned: true,
      delegate: _MonthHeading(
        calendarMonthTitle(month),
        context.colors,
        context.type.uiSmall,
      ),
    ),
    SliverList(
      delegate: SliverChildBuilderDelegate((context, week) {
        final firstDay = calendarAddDays(calendarMonday(month), week * 7);
        return SizedBox(
          height: _cellHeight,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var d = 0; d < 7; d++)
                Expanded(
                  child: _dayCell(context, calendarAddDays(firstDay, d), month),
                ),
            ],
          ),
        );
      }, childCount: _weeks(month)),
    ),
  ];
  Widget _dayCell(BuildContext context, DateTime day, DateTime month) {
    final mobile = context.mobile;
    final items = widget.store.onDay(day);
    final selected = sameDay(day, widget.selected);
    final today = sameDay(day, clockNow());
    return DragTarget<Object>(
      onWillAcceptWithDetails: (details) =>
          _canDropCalendarPayload(details.data),
      onAcceptWithDetails: (details) => widget.onMove(details.data, day),
      builder: (context, candidates, rejected) => Semantics(
        label: '${dayTitle(day)}, ${items.length} items',
        selected: selected,
        button: true,
        child: Container(
          key: ValueKey('calendar-day-${calendarDateKey(day)}-${month.month}'),
          decoration: BoxDecoration(
            color: candidates.isNotEmpty
                ? context.colors.bgSelectedBlock
                : !mobile && day.weekday > 5
                ? context.colors.bgSecondary
                : null,
            border: Border(
              right: BorderSide(
                color: mobile ? const Color(0x00000000) : context.colors.border,
              ),
              bottom: BorderSide(color: context.colors.border),
            ),
          ),
          child: Hover(
            onTap: () => widget.onSelected(context, day),
            selected: selected && !mobile,
            radius: 0,
            child: Padding(
              padding: EdgeInsets.symmetric(horizontal: mobile ? 0 : 4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    height: mobile ? 39 : 32,
                    child: Row(
                      mainAxisAlignment: mobile
                          ? MainAxisAlignment.center
                          : MainAxisAlignment.end,
                      children: [
                        if (!mobile) ...[
                          IconBtn(
                            Ico.plus,
                            size: 22,
                            glyph: 14,
                            semanticLabel:
                                'New task on ${calendarShortDate(day)}',
                            onTap: () => widget.onCreate(day),
                          ),
                          const Spacer(),
                        ],
                        Container(
                          alignment: Alignment.center,
                          width: mobile ? 30 : 26,
                          height: mobile ? 30 : 24,
                          decoration: BoxDecoration(
                            color: today
                                ? context.colors.accent
                                : selected && mobile
                                ? context.colors.bgHover
                                : null,
                            shape: BoxShape.circle,
                            border: selected && mobile && !today
                                ? Border.all(color: context.colors.threadStrong)
                                : null,
                          ),
                          child: Text(
                            '${day.day}',
                            style: tsRaw(
                              mobile ? 16 : 14,
                              20,
                              today ? 700 : 500,
                              today
                                  ? context.colors.onAccent
                                  : day.month != month.month
                                  ? context.colors.textTertiary
                                  : context.colors.textSecondary,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  if (mobile)
                    Row(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        for (final item in items.take(4))
                          Padding(
                            padding: const EdgeInsets.all(1.5),
                            child: _CalendarDot(
                              item: item,
                              settings: widget.store.settings,
                              size: 4,
                            ),
                          ),
                      ],
                    )
                  else ...[
                    for (final item in items.take(3))
                      Padding(
                        padding: const EdgeInsets.only(bottom: 3),
                        child: _CalendarChip(
                          item: item,
                          settings: widget.store.settings,
                          checkbox: widget.store.checkboxes[item.page] ?? true,
                          onOpen: () => widget.onOpen(item),
                          onToggle: () => widget.onToggle(item),
                        ),
                      ),
                    if (items.length > 3)
                      Align(
                        alignment: Alignment.centerLeft,
                        child: TextBtn(
                          '+${items.length - 3} more',
                          height: 16,
                          onTap: () => widget.onSelected(context, day),
                        ),
                      ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _MonthHeading extends SliverPersistentHeaderDelegate {
  _MonthHeading(this.title, this.colors, this.type);
  final String title;
  final AppColors colors;
  final TypeSpec type;
  @override
  double get minExtent => 28;
  @override
  double get maxExtent => 28;
  @override
  Widget build(
    BuildContext context,
    double shrinkOffset,
    bool overlapsContent,
  ) => Container(
    alignment: Alignment.centerLeft,
    decoration: BoxDecoration(
      color: colors.bgPrimary,
      border: Border(bottom: BorderSide(color: colors.border)),
    ),
    child: Text(title, style: ts(type, colors.textSecondary, weight: 700)),
  );
  @override
  bool shouldRebuild(_MonthHeading old) =>
      title != old.title || colors != old.colors || type != old.type;
}

class _CalendarTimeGrid extends StatefulWidget {
  const _CalendarTimeGrid({
    required this.days,
    required this.store,
    required this.onCreate,
    required this.onOpen,
    required this.onToggle,
    required this.onMove,
    required this.onSelect,
  });
  final List<DateTime> days;
  final VaultCalendarStore store;
  final void Function(DateTime day, int? hour) onCreate;
  final ValueChanged<AgendaItem> onOpen, onToggle;
  final _MoveCalendarItem onMove;
  final ValueChanged<DateTime> onSelect;
  @override
  State<_CalendarTimeGrid> createState() => _CalendarTimeGridState();
}

class _CalendarTimeGridState extends State<_CalendarTimeGrid> {
  final _scroll = ScrollController(initialScrollOffset: 480);
  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final now = clockNow();
    final minute = now.hour * 60 + now.minute;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 14),
          child: Row(
            children: [
              const SizedBox(width: 80),
              for (final day in widget.days)
                Expanded(
                  child: Hover(
                    onTap: () => widget.onSelect(day),
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 12),
                      child: Row(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Text(
                            weekdays[day.weekday - 1].substring(0, 3),
                            style: ts(context.type.uiSmall, c.textSecondary),
                          ),
                          const SizedBox(width: 5),
                          Container(
                            width: 27,
                            height: 27,
                            alignment: Alignment.center,
                            decoration: BoxDecoration(
                              shape: BoxShape.circle,
                              color: sameDay(day, now) ? c.accent : null,
                            ),
                            child: Text(
                              '${day.day}',
                              style: tsDisplay(
                                context.type.h3,
                                sameDay(day, now) ? c.onAccent : c.textPrimary,
                              ),
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
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 14),
          decoration: BoxDecoration(
            border: Border(
              top: BorderSide(color: c.border),
              bottom: BorderSide(color: c.border),
            ),
          ),
          constraints: const BoxConstraints(minHeight: 58, maxHeight: 107),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 80,
                child: Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    'All day',
                    style: tsRaw(10, 14, 500, c.textTertiary),
                  ),
                ),
              ),
              for (final day in widget.days)
                Expanded(
                  child: DragTarget<Object>(
                    key: ValueKey('calendar-all-day-${calendarDateKey(day)}'),
                    onWillAcceptWithDetails: (d) =>
                        _canDropCalendarPayload(d.data),
                    onAcceptWithDetails: (d) => widget.onMove(d.data, day),
                    builder: (context, candidates, _) => Container(
                      color: candidates.isNotEmpty ? c.bgSelectedBlock : null,
                      padding: const EdgeInsets.fromLTRB(3, 5, 3, 5),
                      child: SingleChildScrollView(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            for (final item
                                in widget.store
                                    .onDay(day)
                                    .where((i) => !i.hasTime))
                              Padding(
                                padding: const EdgeInsets.only(bottom: 3),
                                child: _CalendarChip(
                                  item: item,
                                  settings: widget.store.settings,
                                  checkbox:
                                      widget.store.checkboxes[item.page] ??
                                      true,
                                  onOpen: () => widget.onOpen(item),
                                  onToggle: () => widget.onToggle(item),
                                ),
                              ),
                            if (!widget.store.onDay(day).any((i) => !i.hasTime))
                              Hover(
                                onTap: () => widget.onCreate(day, null),
                                child: const SizedBox(height: 40),
                              ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
        Expanded(
          child: SingleChildScrollView(
            key: const ValueKey('calendar-time-scroll'),
            controller: _scroll,
            padding: const EdgeInsets.fromLTRB(14, 0, 14, 16),
            child: SizedBox(
              height: 1440,
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: 80,
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        for (var hour = 0; hour < 24; hour++)
                          Positioned(
                            top: hour * 60.0 + (hour == 0 ? 2 : -7),
                            right: 9,
                            child: Text(
                              '${hour.toString().padLeft(2, '0')}:00',
                              style: tsRaw(
                                10,
                                14,
                                400,
                                c.textTertiary,
                              ).copyWith(fontFamily: monoFamily),
                            ),
                          ),
                        if (widget.days.any((d) => sameDay(d, now)))
                          Positioned(
                            key: const ValueKey('calendar-now-label'),
                            top: minute - 8.0,
                            left: 0,
                            child: ColoredBox(
                              color: c.bgPrimary,
                              child: Padding(
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 3,
                                ),
                                child: Text(
                                  'now ${calendarTime(minute)}',
                                  style: tsRaw(10, 16, 700, c.accentText),
                                ),
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                  for (final day in widget.days)
                    Expanded(
                      child: LayoutBuilder(
                        builder: (context, constraints) {
                          final segments = calendarSegments(
                            widget.store.onDay(day),
                            day,
                          );
                          return Container(
                            decoration: BoxDecoration(
                              color: day.weekday > 5 ? c.bgSecondary : null,
                              border: Border(left: BorderSide(color: c.border)),
                            ),
                            child: Stack(
                              children: [
                                for (var hour = 0; hour < 24; hour++)
                                  Positioned(
                                    top: hour * 60.0,
                                    height: 60,
                                    left: 0,
                                    right: 0,
                                    child: DragTarget<Object>(
                                      key: ValueKey(
                                        'calendar-slot-${calendarDateKey(day)}-$hour',
                                      ),
                                      onWillAcceptWithDetails: (d) =>
                                          _canDropCalendarPayload(d.data),
                                      onAcceptWithDetails: (d) => widget.onMove(
                                        d.data,
                                        day,
                                        hour: hour,
                                      ),
                                      builder: (context, candidates, _) =>
                                          Semantics(
                                            label:
                                                'New task ${calendarShortDate(day)} at ${calendarTime(hour * 60)}',
                                            button: true,
                                            child: Hover(
                                              onTap: () =>
                                                  widget.onCreate(day, hour),
                                              radius: 0,
                                              background: candidates.isNotEmpty
                                                  ? c.bgSelectedBlock
                                                  : null,
                                              border: Border(
                                                top: BorderSide(
                                                  color: c.border,
                                                ),
                                              ),
                                              child: const SizedBox.expand(),
                                            ),
                                          ),
                                    ),
                                  ),
                                for (final segment in segments)
                                  Positioned(
                                    key: ValueKey(
                                      'calendar-timed-${segment.item.page}-${calendarDateKey(day)}',
                                    ),
                                    top: segment.startMinute.toDouble(),
                                    height: math
                                        .min(
                                          1440 - segment.startMinute,
                                          math.max(
                                            22,
                                            segment.endMinute -
                                                segment.startMinute,
                                          ),
                                        )
                                        .toDouble(),
                                    left:
                                        constraints.maxWidth *
                                            segment.lane /
                                            segment.lanes +
                                        3,
                                    width: math.max(
                                      1,
                                      constraints.maxWidth / segment.lanes - 6,
                                    ),
                                    child: _timedBlock(context, segment),
                                  ),
                                if (sameDay(day, now))
                                  Positioned(
                                    key: const ValueKey('calendar-now-line'),
                                    top: minute.toDouble(),
                                    left: 0,
                                    right: 0,
                                    child: IgnorePointer(
                                      child: Container(
                                        height: 2,
                                        color: c.accentText,
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                          );
                        },
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _timedBlock(BuildContext context, CalendarTimeSegment segment) {
    final item = segment.item;
    final color = _calendarColor(context, item, widget.store.settings);
    return _calendarDrag(
      context,
      item,
      ClipRect(
        child: Semantics(
          button: true,
          label:
              '${item.title}, ${segment.continued ? 'continued, ' : ''}${calendarTime(segment.startMinute)} to ${calendarTime(segment.endMinute)}',
          child: Hover(
            onTap: () => widget.onOpen(item),
            background: color.withValues(alpha: .24),
            border: Border.all(color: color.withValues(alpha: .6)),
            padding: const EdgeInsets.all(5),
            child: LayoutBuilder(
              builder: (context, size) => Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    '${item.assignee == 'agent' ? 'Agent · ' : ''}${segment.continued ? 'Continued · ' : ''}${calendarTime(segment.startMinute)}${size.maxHeight > 42 ? ' – ${calendarTime(segment.endMinute)}' : ''}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: tsRaw(10, 12, 500, context.colors.textSecondary),
                  ),
                  if (size.maxHeight > 25) ...[
                    const SizedBox(height: 3),
                    Expanded(
                      child: Text(
                        item.title,
                        maxLines: math.max(
                          1,
                          ((size.maxHeight - 15) / 16).floor(),
                        ),
                        overflow: TextOverflow.ellipsis,
                        style:
                            tsRaw(
                              12,
                              16,
                              600,
                              item.done
                                  ? context.colors.textTertiary
                                  : context.colors.textPrimary,
                            ).copyWith(
                              decoration: item.done
                                  ? TextDecoration.lineThrough
                                  : null,
                            ),
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class CalendarJumpPicker extends StatefulWidget {
  const CalendarJumpPicker({super.key, required this.selected});
  final DateTime selected;
  @override
  State<CalendarJumpPicker> createState() => _CalendarJumpPickerState();
}

class _CalendarJumpPickerState extends State<CalendarJumpPicker> {
  late DateTime _month = calendarMonth(widget.selected);
  late final _text = TextEditingController(
    text: calendarDateKey(widget.selected),
  );
  final _focus = FocusNode();
  String? _error;
  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _submit() {
    final text = _text.text.trim();
    final date = DateTime.tryParse(text);
    if (!RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(text) ||
        date == null ||
        calendarDateKey(date) != text) {
      setState(() => _error = 'Use a valid date, YYYY-MM-DD.');
      return;
    }
    Navigator.of(context).pop(calendarDay(date));
  }

  @override
  Widget build(BuildContext context) {
    final first = calendarMonday(_month);
    final weeks = calendarMonthWeeks(_month);
    return Padding(
      padding: const EdgeInsets.all(18),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  'Jump to date',
                  style: tsDisplay(context.type.h3, context.colors.textPrimary),
                ),
              ),
              IconBtn(
                Ico.close,
                semanticLabel: 'Close jump',
                onTap: () => Navigator.of(context).pop(),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              IconBtn(
                Ico.chevronLeft,
                semanticLabel: 'Previous month',
                onTap: () => setState(() => _month = calendarMonth(_month, -1)),
              ),
              Expanded(
                child: Center(
                  child: Text(
                    calendarMonthTitle(_month),
                    style: ts(
                      context.type.ui,
                      context.colors.textPrimary,
                      weight: 700,
                    ),
                  ),
                ),
              ),
              IconBtn(
                Ico.chevronRight,
                semanticLabel: 'Next month',
                onTap: () => setState(() => _month = calendarMonth(_month, 1)),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              for (final day in weekdays)
                Expanded(
                  child: Center(
                    child: Text(
                      day.substring(0, 1),
                      style: ts(
                        context.type.uiSmall,
                        context.colors.textTertiary,
                      ),
                    ),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 4),
          for (var week = 0; week < weeks; week++)
            Row(
              children: [
                for (var n = 0; n < 7; n++)
                  Expanded(
                    child: Builder(
                      builder: (ctx) {
                        final day = calendarAddDays(first, week * 7 + n);
                        return Semantics(
                          label: dayTitle(day),
                          button: true,
                          selected: sameDay(day, widget.selected),
                          child: Hover(
                            onTap: () => Navigator.of(context).pop(day),
                            selected: sameDay(day, widget.selected),
                            child: SizedBox(
                              height: 34,
                              child: Center(
                                child: Text(
                                  '${day.day}',
                                  style: ts(
                                    context.type.uiSmall,
                                    day.month == _month.month
                                        ? context.colors.textPrimary
                                        : context.colors.textTertiary,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        );
                      },
                    ),
                  ),
              ],
            ),
          const SizedBox(height: 12),
          FieldFrame(
            height: 40,
            child: PlainField(
              key: const ValueKey('calendar-jump-date'),
              controller: _text,
              focusNode: _focus,
              style: ts(context.type.ui, context.colors.textPrimary),
              placeholder: 'YYYY-MM-DD',
              onSubmitted: (_) => _submit(),
            ),
          ),
          if (_error != null)
            Text(
              _error!,
              style: ts(context.type.uiSmall, context.colors.inlineCodeText),
            ),
          const SizedBox(height: 12),
          SecondaryButton(label: 'Go to date', onTap: _submit),
        ],
      ),
    );
  }
}
