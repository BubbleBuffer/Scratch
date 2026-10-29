import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../addons/tasks/api.dart' show CalendarJumpPicker;
import '../core/debounce.dart';
import '../core/models.dart';
import '../state/agenda_store.dart';
import '../state/app_state.dart';
import '../ui/ui.dart';

String todayDateKey(DateTime day) =>
    '${day.year.toString().padLeft(4, '0')}-${day.month.toString().padLeft(2, '0')}-${day.day.toString().padLeft(2, '0')}';

/// A lazy strip with dates on both sides of a fixed origin. Browsing the strip
/// never changes the selected day; selection belongs to the shared agenda.
class TodayDateStrip extends StatefulWidget {
  const TodayDateStrip({super.key});

  @override
  State<TodayDateStrip> createState() => _TodayDateStripState();
}

class _TodayDateStripState extends State<TodayDateStrip> {
  final _center = GlobalKey();
  final _focus = FocusNode();
  final _debounce = Debounce();
  late final ScrollController _scroll;
  AppState? _app;
  late DateTime _origin;
  DateTime? _selected;
  double _extent = 58;
  int? _from, _to;
  int _generation = 0;
  bool _jumpOpen = false;
  bool _failed = false;
  Map<int, List<AgendaItem>> _marks = {};
  Set<int> _notes = {};
  CalendarSettings _calendars = const CalendarSettings(
    calendars: [],
    databases: [],
  );

  DateTime _add(DateTime day, int days) =>
      DateTime(day.year, day.month, day.day + days);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = context.app;
    if (_app == null) {
      _app = app;
      _origin = calendarDay(app.agenda.selectedDay);
      _selected = _origin;
      _extent = context.mobile ? 48 : 58;
      _scroll = ScrollController(initialScrollOffset: _extent / 2)
        ..addListener(_scrolled);
      app.feed.addListener(_changed);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _loadVisible();
      });
    } else {
      final nextExtent = context.mobile ? 48.0 : 58.0;
      if (_extent != nextExtent) {
        _extent = nextExtent;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _showSelected(animate: false);
        });
      }
    }
  }

  void _changed(ChangeBatch _) {
    _from = _to = null;
    _generation++;
    _debounce.run(
      const Duration(milliseconds: 150),
      () => unawaited(_loadVisible()),
    );
  }

  void _scrolled() => _debounce.run(
    const Duration(milliseconds: 100),
    () => unawaited(_loadVisible()),
  );

  Future<void> _loadVisible() async {
    final app = _app;
    if (!mounted || app == null || app.vault.isClosed) return;
    final middle = _scroll.hasClients ? (_scroll.offset / _extent).floor() : 0;
    final visible = _scroll.hasClients
        ? (_scroll.position.viewportDimension / _extent).ceil()
        : 14;
    // Keep a small buffer so swiping shows marks without a query per tile.
    final radius = math.max(14, visible + 7);
    final first = _add(_origin, middle - radius);
    final last = _add(_origin, middle + radius + 1);
    final from = dayStamp(first), to = dayStamp(last);
    if (_from != null && from >= _from! && to <= _to!) return;
    _from = from;
    _to = to;
    final generation = ++_generation;
    try {
      await app.vault.flush();
      final results = await Future.wait<Object>([
        app.vault.run(
          (v) => v.agenda(
            timedFromMs: first.millisecondsSinceEpoch,
            timedToMs: last.millisecondsSinceEpoch,
            dayFromMs: from,
            dayToMs: to,
          ),
        ),
        app.vault.dayNotesInRange(fromMs: from, toMs: to),
        app.vault.listCalendars(),
      ]);
      if (!mounted || app.vault.isClosed || generation != _generation) return;
      final marks = <int, List<AgendaItem>>{};
      for (final item in results[0] as List<AgendaItem>) {
        if (item.kind == AgendaKind.mention) continue;
        final day = DateTime.fromMillisecondsSinceEpoch(
          item.startMs.toInt(),
          isUtc: !item.hasTime,
        );
        // Today places a timed item on its start day, including when an
        // event continues past midnight in Calendar.
        marks.putIfAbsent(dayStamp(day), () => []).add(item);
      }
      setState(() {
        _marks = marks;
        _notes = (results[1] as List<int>).toSet();
        _calendars = results[2] as CalendarSettings;
        _failed = false;
      });
    } catch (_) {
      if (!mounted || generation != _generation) return;
      _from = _to = null;
      setState(() => _failed = true);
    }
  }

  void _showSelected({bool animate = true}) {
    if (!_scroll.hasClients) return;
    final offset =
        (dayStamp(_app!.agenda.selectedDay) - dayStamp(_origin)) /
            Duration.millisecondsPerDay *
            _extent +
        _extent / 2;
    final duration = animate ? Motion.of(context).standard : Duration.zero;
    if (duration == Duration.zero) {
      _scroll.jumpTo(offset);
    } else {
      unawaited(
        _scroll.animateTo(offset, duration: duration, curve: Motion.settle),
      );
    }
    unawaited(_loadVisible());
  }

  void _select(DateTime day) {
    _app!.agenda.selectDay(day);
    _focus.requestFocus();
  }

  void _browse(int direction) {
    if (!_scroll.hasClients) return;
    final offset =
        _scroll.offset +
        direction * math.max(200, _scroll.position.viewportDimension * .75);
    final duration = Motion.of(context).standard;
    if (duration == Duration.zero) {
      _scroll.jumpTo(offset);
    } else {
      unawaited(
        _scroll.animateTo(offset, duration: duration, curve: Motion.settle),
      );
    }
    _focus.requestFocus();
  }

  Future<void> _jump() async {
    if (_jumpOpen) return;
    _jumpOpen = true;
    try {
      final date = await showPickerSheet<DateTime>(
        context,
        (_) => CalendarJumpPicker(selected: _app!.agenda.selectedDay),
        desktopWidth: 350,
        desktopHeight: 440,
      );
      if (!mounted) return;
      if (date != null) _select(date);
      _focus.requestFocus();
    } finally {
      _jumpOpen = false;
    }
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is! KeyDownEvent ||
        _jumpOpen ||
        ModalRoute.of(context)?.isCurrent == false) {
      return KeyEventResult.ignored;
    }
    final keyboard = HardwareKeyboard.instance;
    if (keyboard.isControlPressed ||
        keyboard.isAltPressed ||
        keyboard.isMetaPressed) {
      return KeyEventResult.ignored;
    }
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused?.widget is EditableText ||
        focused?.findAncestorWidgetOfExactType<EditableText>() != null) {
      return KeyEventResult.ignored;
    }
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.arrowLeft ||
        key == LogicalKeyboardKey.arrowRight) {
      _select(
        _add(
          _app!.agenda.selectedDay,
          key == LogicalKeyboardKey.arrowLeft ? -1 : 1,
        ),
      );
    } else if (key == LogicalKeyboardKey.keyT) {
      _app!.agenda.backToToday();
      _showSelected();
    } else if (key == LogicalKeyboardKey.keyG) {
      unawaited(_jump());
    } else {
      return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  Color _color(AgendaItem item) {
    String? color = item.spaceColor;
    for (final calendar in _calendars.calendars) {
      if (calendar.id == item.calendar) {
        color = calendar.color;
        break;
      }
    }
    final c = context.colors;
    return spaceColor(color, c.dark ? Brightness.dark : Brightness.light) ??
        c.threadStrong;
  }

  Widget _tile(DateTime day, AgendaStore store) {
    final c = context.colors;
    final selected = sameDay(day, store.selectedDay);
    final today = sameDay(day, store.today);
    final stamp = dayStamp(day);
    final items = _marks[stamp] ?? const <AgendaItem>[];
    final colors = <String, Color>{};
    for (final item in items) {
      colors.putIfAbsent(
        item.calendar ?? item.space ?? item.collection ?? 'default',
        () => _color(item),
      );
      if (colors.length == 3) break;
    }
    final hasNote = _notes.contains(stamp);
    final ink = selected ? c.bgPrimary : c.textPrimary;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 2),
      child: Semantics(
        key: ValueKey('today-date-${todayDateKey(day)}'),
        button: true,
        selected: selected,
        label:
            '${dayTitle(day, year: true)}${today ? ', today' : ''}${items.isNotEmpty ? ', ${items.length} items' : ''}${hasNote ? ', has a note' : ''}',
        child: Hover(
          onTap: () => _select(day),
          lift: 0,
          background: selected ? c.textPrimary : null,
          borderRadius: BorderRadius.circular(999),
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Text(
                weekdays[day.weekday - 1].substring(0, 3),
                style: tsRaw(11, 15, 500, selected ? ink : c.textSecondary),
              ),
              const SizedBox(height: 2),
              Container(
                width: 30,
                height: 30,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  border: today && !selected
                      ? Border.all(color: c.accentText)
                      : null,
                ),
                child: Text(
                  '${day.day}',
                  style: tsDisplay(
                    const TypeSpec(21, 25, 640),
                    today && !selected ? c.accentText : ink,
                  ),
                ),
              ),
              const SizedBox(height: 4),
              SizedBox(
                height: 12,
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    for (final color in colors.values)
                      Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 1.5),
                        child: Container(
                          width: 5,
                          height: 5,
                          decoration: BoxDecoration(
                            color: color,
                            shape: BoxShape.circle,
                          ),
                        ),
                      ),
                    if (hasNote) ...[
                      const SizedBox(width: 3),
                      Icon20(
                        Ico.page,
                        key: ValueKey('today-note-mark-${todayDateKey(day)}'),
                        size: 11,
                        color: selected ? ink : c.iconSecondary,
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<AgendaStore>();
    if (!sameDay(_selected!, store.selectedDay)) {
      _selected = store.selectedDay;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) {
          _showSelected();
        }
      });
    }
    final c = context.colors;
    return Focus(
      key: const ValueKey('today-strip-focus'),
      focusNode: _focus,
      onKeyEvent: _key,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: TextBtn(
                  '${months[store.selectedDay.month - 1]} ${store.selectedDay.year}',
                  key: const ValueKey('today-jump'),
                  onTap: _jump,
                  color: c.textSecondary,
                ),
              ),
              TextBtn(
                'Today',
                key: const ValueKey('today-back'),
                onTap: () {
                  store.backToToday();
                  _showSelected();
                  _focus.requestFocus();
                },
                color: c.accentText,
              ),
            ],
          ),
          const SizedBox(height: 6),
          Row(
            children: [
              IconBtn(
                Ico.chevronLeft,
                key: const ValueKey('today-strip-previous'),
                semanticLabel: 'Earlier dates',
                size: context.mobile ? 36 : 28,
                glyph: 16,
                onTap: () => _browse(-1),
              ),
              Expanded(
                child: SizedBox(
                  height: 82,
                  child: Listener(
                    onPointerSignal: (event) {
                      if (event is PointerScrollEvent &&
                          event.scrollDelta.dy.abs() >
                              event.scrollDelta.dx.abs() &&
                          !isCommandKeyPressed) {
                        GestureBinding.instance.pointerSignalResolver.register(
                          event,
                          (_) {
                            if (_scroll.hasClients) {
                              _scroll.jumpTo(
                                _scroll.offset + event.scrollDelta.dy,
                              );
                            }
                          },
                        );
                      }
                    },
                    child: ScrollConfiguration(
                      behavior: ScrollConfiguration.of(context).copyWith(
                        scrollbars: false,
                        dragDevices: {
                          PointerDeviceKind.touch,
                          PointerDeviceKind.mouse,
                          PointerDeviceKind.stylus,
                          PointerDeviceKind.invertedStylus,
                          PointerDeviceKind.trackpad,
                        },
                      ),
                      child: CustomScrollView(
                        key: const ValueKey('today-date-strip'),
                        controller: _scroll,
                        scrollDirection: Axis.horizontal,
                        center: _center,
                        anchor: .5,
                        slivers: [
                          SliverFixedExtentList(
                            itemExtent: _extent,
                            delegate: SliverChildBuilderDelegate(
                              (_, index) =>
                                  _tile(_add(_origin, -index - 1), store),
                            ),
                          ),
                          SliverFixedExtentList(
                            key: _center,
                            itemExtent: _extent,
                            delegate: SliverChildBuilderDelegate(
                              (_, index) => _tile(_add(_origin, index), store),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              IconBtn(
                Ico.chevronRight,
                key: const ValueKey('today-strip-next'),
                semanticLabel: 'Later dates',
                size: context.mobile ? 36 : 28,
                glyph: 16,
                onTap: () => _browse(1),
              ),
            ],
          ),
          if (_failed)
            TextBtn(
              'Could not load date markers · Retry',
              key: const ValueKey('today-strip-retry'),
              onTap: () => unawaited(_loadVisible()),
              color: c.textSecondary,
            ),
          const SizedBox(height: 10),
          Container(height: 1, color: c.border),
        ],
      ),
    );
  }

  @override
  void dispose() {
    _generation++;
    _app?.feed.removeListener(_changed);
    _debounce.cancel();
    _scroll.dispose();
    _focus.dispose();
    super.dispose();
  }
}
