// The page menu's Share label with the link count: plain "Share" with no
// links, while loading or on error, and "Share · N link(s)" otherwise. The
// label is a pure function, so the menu itself (which needs the bridge) is
// not pumped here; share_menu_test.dart covers the menu entry.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/page/page_menu.dart';

void main() {
  test('the label is plain Share while the count is unknown or zero', () {
    expect(shareMenuLabel(null), 'Share');
    expect(shareMenuLabel(0), 'Share');
  });

  test('the label names one link in the singular', () {
    expect(shareMenuLabel(1), 'Share · 1 link');
  });

  test('the label names more links in the plural', () {
    expect(shareMenuLabel(2), 'Share · 2 links');
    expect(shareMenuLabel(12), 'Share · 12 links');
  });
}
