/// A note dragged from a host surface to an add-on.
///
/// Identity and presentation only: receivers load the current note before
/// deciding what to change. No editor snapshot or host widget is carried.
class NoteDragPayload {
  const NoteDragPayload({
    required this.pageId,
    this.title = '',
    this.pageType = '',
  });

  final String pageId;
  final String title;
  final String pageType;
}
