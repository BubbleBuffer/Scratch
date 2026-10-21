// The timeline view: a bar per row across the days of its date (or date
// range); drag a bar to move it, drag its ends to change the start or end;
// zoom by week, month or quarter. Only the visible stretch of time (plus a
// margin) is queried, and only the visible rows are built.

import 'dart:math' as math;

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../../../core/models.dart';
import '../../../../ui/database/config.dart';
import '../../../../ui/database/db_icons.dart';
import '../../../../ui/database/db_theme.dart';
import '../../../../ui/database/values.dart';
import '../../../../ui/database/widgets.dart';
import '../../../../registry/view_registry.dart';
import '../../../../ui/ui.dart';
import '../calendar/calendar_view.dart' show pickDateProp;
import '../common.dart';
import '../dates.dart';

String _zoomLabel(TimelineZoom z) => switch (z) {
  TimelineZoom.week => 'Week',
  TimelineZoom.month => 'Month',
  TimelineZoom.quarter => 'Quarter',
};

double dayWidthOf(TimelineZoom z) => switch (z) {
  TimelineZoom.week => 120.0,
  TimelineZoom.month => 40.0,
  TimelineZoom.quarter => 14.0,
};

final timelineView = ViewType(
  kind: ViewKind.timeline,
  label: 'Timeline',
  icon: DbIco.timeline,
  needsDate: true,
  build: (context, a) => TimelineView(args: a),
  options: (context, a) => [
    ViewOption(
      label: 'Show timeline by',
      value: menuValue(
        context,
        a.store.def(a.view.dateProp ?? '')?.name ?? 'None',
      ),
      onTap: (ctx, rect) => pickDateProp(ctx, rect, a),
    ),
    ViewOption(
      label: 'Zoom',
      value: menuValue(context, _zoomLabel(a.view.cfg.zoom)),
      onTap: (ctx, rect) => _pickZoom(ctx, rect, a),
    ),
  ],
);

Future<void> _pickZoom(BuildContext ctx, Rect rect, ViewArgs a) async {
  final z = await showPanel<TimelineZoom>(
    ctx,
    rect,
    (c) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final z in TimelineZoom.values)
          MenuItem(
            key: ValueKey('zoom-${z.name}'),
            label: _zoomLabel(z),
            trailing: a.view.cfg.zoom == z
                ? Icon20(Ico.check, size: 16, color: c.colors.iconPrimary)
                : null,
            onTap: () => Navigator.of(c).pop(z),
          ),
      ],
    ),
    width: 200,
  );
  if (z != null) await a.store.updateConfig(a.view, (c) => c.zoom = z);
}

/// Days before and after today the timeline can scroll to.
const _span = 540;

class TimelineView extends StatefulWidget {
  const TimelineView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<TimelineView> createState() => _TimelineViewState();
}

class _TimelineViewState extends State<TimelineView> {
  final _h = ScrollController();
  final _v = ScrollController();
  final Day _origin = Day.of(clockNow()).add(-_span);
  double _viewport = 1000;
  TimelineZoom? _zoom;
  bool _placed = false;

  /// Bars being dragged: row id -> (start shift, end shift) in pixels.
  final Map<String, (double, double)> _drag = {};

  ViewArgs get a => widget.args;
  double get _dw => dayWidthOf(a.view.cfg.zoom);
  double get _width => (2 * _span + 1) * _dw;

  @override
  void initState() {
    super.initState();
    _h.addListener(_onScroll);
  }

  @override
  void dispose() {
    _h.removeListener(_onScroll);
    _h.dispose();
    _v.dispose();
    super.dispose();
  }

  void _onScroll() {
    _query();
    setState(() {});
  }

  double get _offset => _h.hasClients ? _h.offset : _todayOffset();

  double _todayOffset() =>
      math.max(0, (Day.of(clockNow()) - _origin) * _dw - _viewport / 3);

  /// Queries the visible days plus a margin, in 30-day steps.
  void _query() {
    final key = a.view.dateProp;
    if (key == null) return;
    final firstVisible = (_offset / _dw).floor();
    final lastVisible = ((_offset + _viewport) / _dw).ceil();
    const step = 30;
    final from = ((firstVisible - step) / step).floor() * step;
    final to = ((lastVisible + step) / step).ceil() * step;
    final (f, t) = queryRangeOf(_origin.add(from), _origin.add(to));
    a.store.showRange(key, f, t);
  }

  void _jumpToday() {
    if (_h.hasClients) {
      _h.animateTo(
        _todayOffset(),
        duration: Motion.of(context).standard,
        curve: Motion.easing,
      );
    }
  }

  void _page(int dir) {
    if (_h.hasClients) {
      _h.animateTo(
        (_h.offset + dir * _viewport * 0.6).clamp(
          0,
          _h.position.maxScrollExtent,
        ),
        duration: Motion.of(context).standard,
        curve: Motion.easing,
      );
    }
  }

  Future<void> _commit(RangeEntry e, double ds, double de) async {
    final key = a.view.dateProp!;
    final sd = (ds / _dw).round(), ed = (de / _dw).round();
    if (sd == 0 && ed == 0) return;
    var v = shiftedValue(e, sd, ed);
    // A resized end can't go before the start.
    final end = v.endMs;
    if (end != null && end < v.startMs) {
      v = Values.date(v.startMs, hasTime: v.hasTime);
    }
    await a.store.setValue(e.page.id, key, v);
  }

  @override
  Widget build(BuildContext context) {
    final key = a.view.dateProp;
    final c = context.colors;
    if (key == null || a.store.def(key) == null) {
      return Padding(
        padding: EdgeInsets.fromLTRB(a.gutter, 24, a.gutter, 24),
        child: Builder(
          builder: (b) => ChipBtn(
            key: const ValueKey('timeline-pick-date'),
            label: 'Choose a date property',
            icon: DbIco.date,
            onTap: () => pickDateProp(b, globalRectOf(b), a),
          ),
        ),
      );
    }
    if (_zoom != a.view.cfg.zoom) {
      // Zoom changed: keep the same day at the left edge.
      final day = _zoom == null ? null : (_offset / dayWidthOf(_zoom!));
      _zoom = a.view.cfg.zoom;
      if (day != null && _h.hasClients) {
        SchedulerBinding.instance.addPostFrameCallback((_) {
          if (_h.hasClients) {
            _h.jumpTo((day * _dw).clamp(0, _h.position.maxScrollExtent));
          }
        });
      }
    }

    final entries =
        [
          for (final e in a.store.rangeEntries)
            if (e.prop == key) e,
        ]..sort((x, y) {
          final s = x.startMs.compareTo(y.startMs);
          return s != 0 ? s : x.page.title.compareTo(y.page.title);
        });
    final m = context.mobile;
    const rowH = 36.0;

    return LayoutBuilder(
      builder: (context, box) {
        final viewport = box.maxWidth - (a.inline ? a.gutter : 0);
        if (viewport != _viewport) _viewport = viewport;
        if (!_placed) {
          _placed = true;
          _query();
          SchedulerBinding.instance.addPostFrameCallback((_) {
            if (_h.hasClients) _h.jumpTo(_todayOffset());
          });
        }
        final off = _offset;
        final firstDay = _origin.add((off / _dw).floor());

        final toolbar = Padding(
          padding: EdgeInsets.fromLTRB(
            a.inline ? a.gutter : a.gutter,
            4,
            a.inline ? 8 : a.gutter,
            0,
          ),
          child: SizedBox(
            height: m ? 40 : 30,
            child: Row(
              children: [
                Text(
                  '${monthName(firstDay.month)} ${firstDay.year}',
                  key: const ValueKey('timeline-month'),
                  style: ts(context.type.ui, c.textPrimary, weight: 600),
                ),
                const Spacer(),
                Builder(
                  builder: (b) => Hover(
                    key: const ValueKey('timeline-zoom'),
                    onTap: () => _pickZoom(b, globalRectOf(b), a),
                    radius: Radii.md,
                    padding: const EdgeInsets.symmetric(horizontal: 6),
                    child: SizedBox(
                      height: 24,
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Text(
                            _zoomLabel(a.view.cfg.zoom),
                            style: ts(
                              context.type.ui,
                              c.textTertiary,
                              weight: 400,
                            ),
                          ),
                          const SizedBox(width: 2),
                          Icon20(
                            Ico.chevronDown,
                            size: 14,
                            color: c.iconTertiary,
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
                IconBtn(
                  Ico.chevronLeft,
                  size: 24,
                  glyph: 16,
                  color: c.iconSecondary,
                  semanticLabel: 'Earlier',
                  onTap: () => _page(-1),
                ),
                TextBtn(
                  'Today',
                  key: const ValueKey('timeline-today'),
                  height: 24,
                  color: c.textSecondary,
                  onTap: _jumpToday,
                ),
                IconBtn(
                  Ico.chevronRight,
                  size: 24,
                  glyph: 16,
                  color: c.iconSecondary,
                  semanticLabel: 'Later',
                  onTap: () => _page(1),
                ),
              ],
            ),
          ),
        );

        Widget lane(int i) {
          if (i >= entries.length) {
            return const SizedBox(height: rowH);
          }
          final e = entries[i];
          return SizedBox(
            height: rowH,
            width: _width,
            child: Stack(
              clipBehavior: Clip.none,
              children: [_bar(context, e, rowH)],
            ),
          );
        }

        final lanesH = math.max(entries.length + 1, a.inline ? 4 : 0) * rowH;
        final Widget lanes = a.inline
            ? SizedBox(
                height: lanesH,
                child: Column(
                  children: [
                    for (var i = 0; i < entries.length + 1; i++) lane(i),
                  ],
                ),
              )
            : ListView.builder(
                key: const ValueKey('timeline-rows'),
                controller: _v,
                itemExtent: rowH,
                padding: const EdgeInsets.only(bottom: 80),
                itemCount: entries.length + 1,
                itemBuilder: (context, i) => lane(i),
              );

        final canvas = SizedBox(
          width: _width,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: a.inline ? MainAxisSize.min : MainAxisSize.max,
            children: [
              SizedBox(
                height: 48,
                child: CustomPaint(
                  painter: _HeaderPainter(
                    origin: _origin,
                    dw: _dw,
                    offset: off,
                    viewport: _viewport,
                    colors: c,
                    today: Day.of(clockNow()),
                  ),
                ),
              ),
              if (a.inline)
                CustomPaint(
                  painter: _GridPainter(
                    origin: _origin,
                    dw: _dw,
                    offset: off,
                    viewport: _viewport,
                    colors: c,
                    today: Day.of(clockNow()),
                  ),
                  child: lanes,
                )
              else
                Expanded(
                  child: CustomPaint(
                    painter: _GridPainter(
                      origin: _origin,
                      dw: _dw,
                      offset: off,
                      viewport: _viewport,
                      colors: c,
                      today: Day.of(clockNow()),
                    ),
                    child: lanes,
                  ),
                ),
            ],
          ),
        );

        final scroller = SingleChildScrollView(
          key: const ValueKey('timeline-h'),
          controller: _h,
          scrollDirection: Axis.horizontal,
          child: canvas,
        );

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: a.inline ? MainAxisSize.min : MainAxisSize.max,
          children: [
            toolbar,
            if (a.inline)
              Padding(
                padding: EdgeInsets.only(left: a.gutter),
                child: scroller,
              )
            else
              Expanded(child: scroller),
          ],
        );
      },
    );
  }

  Widget _bar(BuildContext context, RangeEntry e, double rowH) {
    final c = context.colors;
    final s = entryStart(e), f = entryEnd(e);
    final (ds, de) = _drag[e.page.id] ?? (0.0, 0.0);
    final left = (s - _origin) * _dw + ds + 2;
    final width = math.max(_dw * 0.5, (f - s + 1) * _dw + de - ds - 4);
    final title = displayTitle(e.page.title, firstLine: untitledLine(context, e.page));
    final narrow = width < 120;
    final label = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        PageIcon(icon: e.page.icon, size: 16, color: c.iconSecondary),
        const SizedBox(width: 6),
        Flexible(
          child: Text(
            title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: tsRaw(14, 20, 500, c.textPrimary),
          ),
        ),
      ],
    );

    Widget handle(bool start) => Positioned(
      left: start ? 0 : null,
      right: start ? null : 0,
      top: 0,
      bottom: 0,
      width: 8,
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: GestureDetector(
          key: ValueKey('${start ? 'bar-start' : 'bar-end'}-${e.page.title}'),
          behavior: HitTestBehavior.opaque,
          onHorizontalDragUpdate: (d) => setState(() {
            final (a0, b0) = _drag[e.page.id] ?? (0.0, 0.0);
            _drag[e.page.id] = start
                ? (a0 + d.delta.dx, b0)
                : (a0, b0 + d.delta.dx);
          }),
          onHorizontalDragEnd: (_) => _end(e),
        ),
      ),
    );

    return Positioned(
      left: left,
      top: (rowH - 26) / 2,
      height: 26,
      width: width + (narrow ? 400 : 0),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned(
            left: 0,
            top: 0,
            bottom: 0,
            width: width,
            child: GestureDetector(
              key: ValueKey('bar-${e.page.title}'),
              onTap: () =>
                  a.ctl.openRow(context, e.page.id, title: e.page.title),
              onHorizontalDragUpdate: (d) => setState(() {
                final (a0, b0) = _drag[e.page.id] ?? (0.0, 0.0);
                _drag[e.page.id] = (a0 + d.delta.dx, b0 + d.delta.dx);
              }),
              onHorizontalDragEnd: (_) => _end(e),
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 8),
                decoration: BoxDecoration(
                  color: eventPillFill(c, a.store.resolvedSpaceColor),
                  borderRadius: Radii.pill,
                  boxShadow: c.cardShadow(
                    c.cardRing(null),
                    lifted: _drag.containsKey(e.page.id),
                  ),
                ),
                alignment: Alignment.centerLeft,
                child: narrow ? null : label,
              ),
            ),
          ),
          if (narrow)
            Positioned(
              left: width + 8,
              top: 0,
              bottom: 0,
              child: IgnorePointer(child: Center(child: label)),
            ),
          handle(true),
          Positioned(
            left: width - 8,
            top: 0,
            bottom: 0,
            width: 8,
            child: Stack(children: [handle(false)]),
          ),
        ],
      ),
    );
  }

  Future<void> _end(RangeEntry e) async {
    final d = _drag[e.page.id];
    if (d == null) return;
    await _commit(e, d.$1, d.$2);
    if (mounted) setState(() => _drag.remove(e.page.id));
  }
}

/// Month and day labels above the timeline (only the visible part).
class _HeaderPainter extends CustomPainter {
  _HeaderPainter({
    required this.origin,
    required this.dw,
    required this.offset,
    required this.viewport,
    required this.colors,
    required this.today,
  });
  final Day origin;
  final double dw, offset, viewport;
  final AppColors colors;
  final Day today;

  void _text(
    Canvas canvas,
    String s,
    Offset at,
    TextStyle style, {
    bool center = false,
  }) {
    final tp = TextPainter(
      text: TextSpan(text: s, style: style),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, center ? at - Offset(tp.width / 2, 0) : at);
  }

  @override
  void paint(Canvas canvas, Size size) {
    final first = (offset / dw).floor() - 1;
    final last = ((offset + viewport) / dw).ceil() + 1;
    final small = TextStyle(
      fontFamily: sansFamily,
      fontSize: 12,
      height: 1.3,
      color: colors.textTertiary,
    );
    final month = TextStyle(
      fontFamily: sansFamily,
      fontSize: 14,
      height: 1.3,
      fontWeight: FontWeight.w600,
      color: colors.textPrimary,
    );
    final step = dw >= 30 ? 1 : 7;
    for (var i = first; i <= last; i++) {
      final d = origin.add(i);
      final x = i * dw;
      if (d.day == 1) {
        _text(canvas, monthName(d.month), Offset(x + 4, 2), month);
      }
      if (d == today) {
        canvas.drawCircle(
          Offset(x + dw / 2, 34),
          11,
          Paint()..color = colors.textPrimary,
        );
        _text(
          canvas,
          '${d.day}',
          Offset(x + dw / 2, 26),
          small.copyWith(color: colors.bgPrimary, fontWeight: FontWeight.w600),
          center: true,
        );
      } else if (step == 1 || d.weekday0Sunday() == 1) {
        _text(canvas, '${d.day}', Offset(x + dw / 2, 26), small, center: true);
      }
    }
    canvas.drawLine(
      Offset(offset, size.height - 0.5),
      Offset(offset + viewport, size.height - 0.5),
      Paint()..color = colors.calendarLine,
    );
  }

  @override
  bool shouldRepaint(_HeaderPainter o) =>
      o.offset != offset ||
      o.dw != dw ||
      o.viewport != viewport ||
      o.colors != colors;
}

/// Weekend shading and the today line behind the bars.
class _GridPainter extends CustomPainter {
  _GridPainter({
    required this.origin,
    required this.dw,
    required this.offset,
    required this.viewport,
    required this.colors,
    required this.today,
  });
  final Day origin;
  final double dw, offset, viewport;
  final AppColors colors;
  final Day today;

  @override
  void paint(Canvas canvas, Size size) {
    final first = (offset / dw).floor() - 1;
    final last = ((offset + viewport) / dw).ceil() + 1;
    final shade = Paint()..color = colors.bgSecondary;
    if (dw >= 14) {
      for (var i = first; i <= last; i++) {
        final wd = origin.add(i).weekday0Sunday();
        if (wd == 0 || wd == 6) {
          canvas.drawRect(Rect.fromLTWH(i * dw, 0, dw, size.height), shade);
        }
      }
    }
    final t = (today - origin) * dw + dw / 2;
    canvas.drawLine(
      Offset(t, 0),
      Offset(t, size.height),
      Paint()
        ..color = colors.accentText
        ..strokeWidth = 1,
    );
    canvas.drawCircle(Offset(t, 0), 3, Paint()..color = colors.accentText);
    canvas.drawLine(
      Offset(offset, size.height - 0.5),
      Offset(offset + viewport, size.height - 0.5),
      Paint()..color = colors.calendarLine,
    );
  }

  @override
  bool shouldRepaint(_GridPainter o) =>
      o.offset != offset ||
      o.dw != dw ||
      o.viewport != viewport ||
      o.colors != colors;
}
