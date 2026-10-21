// The database's own layouts: table, timeline, calendar, list and gallery.
// The board layout is an add-on of its own (lib/addons/board/). The layout
// picker's order is set in the view registry, not here.

import '../../../registry/view_registry.dart';
import 'calendar/calendar_view.dart';
import 'gallery/gallery_view.dart';
import 'list/list_view.dart';
import 'table/table_view.dart';
import 'timeline/timeline_view.dart';

void registerBuiltinViews(ViewRegistry r) {
  for (final t in [tableView, timelineView, calendarView, listView, galleryView]) {
    r.register(t);
  }
}
