package hara.truffle;

import hara.truffle.InstrumentationModel.EventProjection;
import hara.truffle.InstrumentationModel.ProjectionRequest;

/** Lazy, bounded inspection supplied by the authoritative execution producer. */
interface InstrumentationEventAccess {
  EventProjection project(ProjectionRequest request);

  static InstrumentationEventAccess none() {
    return request -> EventProjection.none();
  }
}
