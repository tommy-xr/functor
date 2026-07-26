// The smallest external store a React island can subscribe to with
// `useSyncExternalStore`: the page's imperative code (the player bridge, the
// example loader) keeps calling plain functions, and the view re-renders.
// A `set` replaces the snapshot object, so identity comparison is the change
// signal.

export interface Store<T> {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => T;
  set: (next: T) => void;
}

export const createStore = <T>(initial: T): Store<T> => {
  let snapshot = initial;
  const listeners = new Set<() => void>();
  return {
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => snapshot,
    set: (next) => {
      snapshot = next;
      for (const listener of listeners) listener();
    },
  };
};
