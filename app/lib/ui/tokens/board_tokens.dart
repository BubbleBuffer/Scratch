// Sizes of the shared board renderer (boards of database pages, kanban and
// task cards).

abstract final class BoardSize {
  /// A card's place while its row loads.
  static const double placeholderCard = 60;

  /// How far a card's tilt turns it while it is dragged, in radians.
  static const double dragTilt = .02;

  /// The kanban card panel's fact labels, and the runner picker with its
  /// runner tile.
  static const double factLabel = 72, runnerPicker = 300, runnerTile = 30;
}
