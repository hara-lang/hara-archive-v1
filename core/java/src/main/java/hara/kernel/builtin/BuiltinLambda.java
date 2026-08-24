package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.Fn;
import hara.lang.protocol.IFn;
import hara.lang.protocol.IPair;
import hara.lang.base.Iter;
import hara.lang.base.fn.Comp;
import hara.lang.base.fn.Partial;
import hara.lang.base.primitive.Array;
import hara.lang.data.*;
import hara.lang.data.Map.Standard;
import hara.lang.data.Tuple.*;
import hara.lang.protocol.ILinearType;

import java.util.Iterator;
import java.util.Map.Entry;
import java.util.function.Function;

import static hara.kernel.base.Module.ReduceInit.*;
import static hara.kernel.base.Module.ReduceType.*;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "lambda")
public interface BuiltinLambda {
  @Module.Fn(name = "apply", vargs = true, complete = true)
  public static <R, FN, ITR> R apply(FN f, ITR vargs) {
    return apply(Fn.toFn(f), vargs);
  }

  @Module.Fn(name = "apply", vargs = true, helper = true)
  public static <R, ITR> R apply(IFn f, ITR vargs) {
    Object[] args = Array.toArray(vargs);
    var lit = Iter.iter(args[args.length - 1]);
    var it = Iter.concat(Array.toIter(args, 0, args.length - 1), lit);
    return (R) f.apply(it);
  }

  @Module.Fn(name = "call", vargs = true, complete = true)
  public static <R, ANY, FN, ITR> R call(ANY o, FN f, ITR vargs) {
    return call(o, Fn.toFn(f), vargs);
  }

  @Module.Fn(name = "call", vargs = true, helper = true)
  public static <R, ANY, ITR> R call(ANY o, IFn f, ITR vargs) {
    Object[] arr = Array.toArray(Iter.concat(Iter.objects(o), Iter.iter(vargs)));
    return (R) f.apply(arr);
  }

  @Module.Fn(name = "comp", vargs = true, complete = true)
  public static <FN, ITR> IFn comp(ITR fns) {
    return new Comp(fns);
  }

  @Module.Fn(name = "F", vargs = true, complete = true)
  public static <ITR> Boolean F(ITR vargs) {
    return false;
  }

  @Module.Fn(name = "group-by", complete = true)
  public static <FN, ITR> Map.Standard groupBy(FN f, ITR source) {
    return groupBy(Fn.toFn(f), Fn.toFn((Function) Function.identity()), source);
  }

  @Module.Fn(name = "group-by", helper = true)
  public static <ITR, K> Map.Standard<K, List> groupBy(IFn fk, IFn fv, ITR source) {
    return (Standard<K, List>)
        Iter.reduceIn(
            Iter.iter(source),
            (Map<K, List>) Map.Standard.EMPTY,
            (m, e) -> {
              K key = (K) fk.invoke(e);
              List v = m.lookup(key);
              if (v == null) {
                v = List.Standard.EMPTY;
              }
              return (Map<K, List>) m.assoc(key, (List) v.conj(fv.invoke(e)));
            });
  }

  @Module.Fn(name = "group-by", complete = true)
  public static <FN, ITR> Map.Standard groupBy(IPair<FN, FN> f, ITR source) {
    return groupBy(Fn.toFn(f.getKey()), Fn.toFn(f.getValue()), source);
  }

  @Module.Fn(name = "identity", complete = true)
  public static <ANY> ANY identity(ANY x) {
    return x;
  }

  @Module.Fn(name = "juxt", vargs = true, complete = true)
  public static <FN, ITR> IFn<Iterator, Iterator, Object> juxt(ITR fns) {
    var jl = Iter.toArray(Iter.map(Iter.iter(fns), Fn::toFn));
    return Fn.toFn(
        (Function)
            (e) -> BuiltinStruct.vector(Iter.map(Iter.iter(jl), (f) -> ((IFn) f).invoke(e))));
  }

  @Module.Fn(name = "keep", complete = true)
  public static <FN> IFn<Iterator, Iterator, Object> keep(FN f) {
    return keep(Fn.toFn(f));
  }

  @Module.Fn(name = "keep", complete = true)
  public static <FN, ITR> Iterator keep(FN f, ITR source) {
    return keep(Fn.toFn(f), source);
  }

  @Module.Fn(name = "keep", helper = true)
  public static IFn<Iterator, Iterator, Object> keep(IFn f) {
    return Fn.toFn((Function) (source) -> keep(f, source));
  }

  @Module.Fn(name = "keep", helper = true)
  public static <ITR> Iterator keep(IFn f, ITR source) {
    var it = Iter.iter(source);
    return Iter.from(it::hasNext, () -> f.invoke(it.next()));
  }

  @Module.Fn(name = "map", complete = true)
  public static <FN> IFn<Iterator, Iterator, Object> map(FN f) {
    return map(Fn.toFn(f));
  }

  @Module.Fn(name = "map", complete = true)
  public static <FN, ITR> Iterator map(FN f, ITR source) {
    return map(Fn.toFn(f), source);
  }

  @Module.Fn(name = "map", helper = true)
  public static IFn<Iterator, Iterator, Object> map(IFn f) {
    return Fn.toFn((Function) (source) -> map(f, source));
  }

  @Module.Fn(name = "map", helper = true)
  public static <ITR> Iterator map(IFn f, ITR source) {
    var it = Iter.iter(source);
    return Iter.from(it::hasNext, () -> f.invoke(it.next()));
  }

  @Module.Fn(name = "map:apply", complete = true)
  public static <FN> IFn<Iterator, Iterator, Object> mapApply(FN f) {
    return mapApply(Fn.toFn(f));
  }

  @Module.Fn(name = "map:apply", complete = true)
  public static <FN, ITR> Iterator mapApply(FN f, ITR source) {
    return mapApply(Fn.toFn(f), source);
  }

  @Module.Fn(name = "map:apply", helper = true)
  public static IFn<Iterator, Iterator, Object> mapApply(IFn f) {
    return Fn.toFn((Function) (source) -> mapApply(f, source));
  }

  @Module.Fn(name = "map:apply", helper = true)
  public static <ITR> Iterator mapApply(IFn f, ITR source) {
    var it = Iter.iter(source);
    return Iter.from(it::hasNext, () -> f.apply(it.next()));
  }

  @Module.Fn(name = "mapcat", complete = true)
  public static <FN> IFn<Iterator, Iterator, Object> mapcat(FN f) {
    return mapcat(Fn.toFn(f));
  }

  @Module.Fn(name = "mapcat", complete = true)
  public static <FN, ITR> Iterator mapcat(FN f, ITR source) {
    return mapcat(Fn.toFn(f), source);
  }

  @Module.Fn(name = "mapcat", helper = true)
  public static IFn<Iterator, Iterator, Object> mapcat(IFn f) {
    return Fn.toFn((Function) (source) -> mapcat(f, source));
  }

  @Module.Fn(name = "mapcat", helper = true)
  public static <ITR> Iterator mapcat(IFn f, ITR source) {
    var it = Iter.iter(source);
    return Iter.mapcat(it, (e) -> (Iterator) f.invoke(e));
  }

  @Module.Fn(name = "map:entries", complete = true)
  public static <FN, ITR> Map.Standard mapEntries(FN f, ITR source) {
    return mapEntries(Fn.toFn(f), source);
  }

  @Module.Fn(name = "map:entries", helper = true)
  public static <ITR> Map.Standard mapEntries(IFn f, ITR source) {
    Iterator<Entry> it = Iter.iter(source);
    return Map.Standard.into(Iter.map(it, e -> (Entry) f.invoke(e)));
  }

  @Module.Fn(name = "map:juxt", helper = true)
  public static <ITR> Map.Standard mapJuxt(IFn fk, IFn fv, ITR source) {
    var it = Iter.iter(source);
    return Map.Standard.into(Iter.map(it, e -> BuiltinStruct.pair(fk.invoke(e), fv.invoke(e))));
  }

  @Module.Fn(name = "map:juxt", complete = true)
  public static <FN, ITR> Map.Standard mapJuxt(IPair<FN, FN> f, ITR source) {
    return mapJuxt(Fn.toFn(f.getKey()), Fn.toFn(f.getValue()), source);
  }

  @Module.Fn(name = "map:keys", complete = true)
  public static <FN, ITR> Map.Standard mapKeys(FN f, ITR source) {
    return mapKeys(Fn.toFn(f), source);
  }

  @Module.Fn(name = "map:keys", helper = true)
  public static <ITR> Map.Standard mapKeys(IFn f, ITR source) {
    Iterator<Entry> it = Iter.iter(source);
    return Map.Standard.into(
        Iter.map(it, e -> BuiltinStruct.pair(f.invoke(e.getKey()), e.getValue())));
  }

  @Module.Fn(name = "map:vals", complete = true)
  public static <FN, ITR> Map.Standard mapVals(FN f, ITR source) {
    return mapVals(Fn.toFn(f), source);
  }

  @Module.Fn(name = "map:vals", helper = true)
  public static <ITR> Map.Standard mapVals(IFn f, ITR source) {
    Iterator<Entry> it = Iter.iter(source);
    return Map.Standard.into(
        Iter.map(it, e -> BuiltinStruct.pair(e.getKey(), f.invoke(e.getValue()))));
  }

  @Module.Fn(name = "NIL", vargs = true, complete = true)
  public static <ITR> Object NIL(ITR x) {
    return null;
  }

  @Module.Fn(name = "partial", vargs = true, complete = true)
  public static <FN, ITR> IFn partial(FN f, ITR vargs) {
    return new Partial(f, vargs);
  }

  @Module.Fn(name = "partition:pair", helper = true)
  public static IFn<Iterator, Iterator, Object> partitionPair() {
    return Fn.toFn((Function) (s) -> partitionPair(s));
  }

  @Module.Fn(name = "partition:pair", complete = true)
  public static <ITR> Iterator partitionPair(ITR source) {
    var it = Iter.iter(source);
    return Iter.partitionPair(it);
  }

  @Module.Fn(name = "pipe", vargs = true, complete = true)
  public static <FN, ITR> IFn<Iterator, Iterator, Object> pipe(ITR fns) {
    var pl = Iter.toArray(Iter.map(Iter.iter(fns), Fn::toFn));
    return Fn.toFn(
        (Function) (it) -> Array.reduce((i, f) -> (Iterator) ((IFn) f).invoke(i), it, pl));
  }

  @Module.Fn(name = "reduce", complete = true)
  public static <ITR, FN, R> R reduce(FN f, FN end, R init, ITR source) {
    return reduce(Fn.toFn(f), Fn.toFn(end), init, source);
  }

  @Module.Fn(name = "reduce", complete = true)
  public static <ITR, FN, R> R reduce(FN f, ITR source) {
    return reduce(Fn.toFn(f), source);
  }

  @Module.Fn(name = "reduce", complete = true)
  public static <ITR, FN, R> R reduce(FN f, R init, ITR source) {
    return reduce(Fn.toFn(f), init, source);
  }

  @Module.Fn(name = "reduce", helper = true)
  public static <ITR, R> R reduce(IFn f, IFn end, R init, ITR source) {
    var it = Iter.iter(source);
    return (R)
        Iter.reduce(it, init, (acc, e) -> (R) f.invoke(acc, e), (acc) -> (Boolean) end.invoke(acc));
  }

  @Module.Fn(name = "reduce", helper = true)
  public static <ITR, R> R reduce(IFn f, ITR source) {
    var it = Iter.iter(source);
    return (R) Iter.reduce(it, (acc, e) -> f.invoke(acc, e));
  }

  @Module.Fn(name = "reduce", helper = true)
  public static <ITR, R> R reduce(IFn f, R init, ITR source) {
    var it = Iter.iter(source);
    return (R) Iter.reduce(it, init, (acc, e) -> (R) f.invoke(acc, e));
  }

  @Module.Fn(name = "reduce-in", complete = true)
  public static <ITR, FN, R> R reduceIn(R init, FN f, ITR source) {
    return reduceIn(init, Fn.toFn(f), source);
  }

  @Module.Fn(name = "reduce-in", helper = true)
  public static <ITR, R> R reduceIn(R init, IFn f, ITR source) {
    var it = Iter.iter(source);
    return (R) Iter.reduceIn(it, init, (acc, e) -> (R) f.invoke(acc, e));
  }

  @Module.Fn(name = "T", vargs = true, complete = true)
  public static <ITR> Boolean T(ITR vargs) {
    return true;
  }
}
