export declare function pipe<A, B>(value: A, fn1: (arg: A) => B): B

export declare function pipe<A, B, C>(value: A, fn1: (arg: A) => B, fn2: (arg: B) => C): C

export declare function pipe<A, B, C, D>(
  value: A,
  fn1: (arg: A) => B,
  fn2: (arg: B) => C,
  fn3: (arg: C) => D
): D

export declare function pipe<A, B, C, D, E>(
  value: A,
  fn1: (arg: A) => B,
  fn2: (arg: B) => C,
  fn3: (arg: C) => D,
  fn4: (arg: D) => E
): E

export declare function pipe<A, B, C, D, E, F>(
  value: A,
  fn1: (arg: A) => B,
  fn2: (arg: B) => C,
  fn3: (arg: C) => D,
  fn4: (arg: D) => E,
  fn5: (arg: E) => F
): F

export declare function pipe<A, B, C, D, E, F, G>(
  value: A,
  fn1: (arg: A) => B,
  fn2: (arg: B) => C,
  fn3: (arg: C) => D,
  fn4: (arg: D) => E,
  fn5: (arg: E) => F,
  fn6: (arg: F) => G
): G
