package hara.lang.base;

import hara.lang.protocol.IExInfo;
import hara.lang.protocol.IMetadata;

import java.util.NoSuchElementException;
import java.util.ArrayList;
import java.util.List;

public interface Ex {

  @SuppressWarnings("serial")
  public class Arity extends IllegalArgumentException {

    public final int actual;

    public final String name;

    public Arity(int actual, String name) {
      this(actual, name, null);
    }

    public Arity(int actual, String name, Throwable cause) {
      super("Ex.Arity - Wrong number of args (" + actual + ") passed to: " + name, cause);
      this.actual = actual;
      this.name = name;
    }
  }

  @SuppressWarnings("serial")
  public class Unsupported extends UnsupportedOperationException {

    public Unsupported() {
      super("Not Supported");
    }

    public Unsupported(String message) {
      super(message);
    }
  }

  @SuppressWarnings("serial")
  public class Info extends RuntimeException implements IExInfo {
    public final IMetadata data;
    private Site createdAt;
    private final ArrayList<Site> throwsAt = new ArrayList<>();

    public record Site(String namespace, String resource, long line, long column) {}

    public Info(String s, IMetadata data) {
      this(s, data, null);
    }

    public Info(String s, IMetadata data, Throwable throwable) {
      super(s, throwable);
      if (data != null) {
        this.data = data;
      } else {
        throw new IllegalArgumentException("Additional data must be non-nil.");
      }
    }

    public Info(String s, IMetadata data, Throwable throwable, Site createdAt, List<Site> throwsAt) {
      this(s, data, throwable);
      this.createdAt = createdAt;
      if (throwsAt != null) this.throwsAt.addAll(throwsAt);
    }

    @Override
    public IMetadata getData() {
      return data;
    }

    public synchronized void recordThrow(Site site) {
      if (site == null) return;
      throwsAt.add(site);
    }

    public synchronized void recordCreation(Site site) {
      if (createdAt == null && site != null) createdAt = site;
    }

    public synchronized Site createdAt() {
      return createdAt;
    }

    public synchronized List<Site> throwsAt() {
      return List.copyOf(throwsAt);
    }

    @Override
    public String toString() {
      return "Ex.Info - " + getMessage() + " " + data.toString();
    }
  }

  @SuppressWarnings("serial")
  public class Syntax extends RuntimeException {
    public Syntax() {}

    public Syntax(String message) {
      super(message);
    }

    public Syntax(String message, Throwable cause) {
      super(message, cause);
    }

    public Syntax(Throwable cause) {
      super(cause);
    }

    public Syntax(
        String message, Throwable cause, boolean enableSuppression, boolean writableStackTrace) {
      super(message, cause, enableSuppression, writableStackTrace);
    }
  }

  @SuppressWarnings("serial")
  public class UndefinedSymbol extends RuntimeException {
    public UndefinedSymbol() {}

    public UndefinedSymbol(String message) {
      super(message);
    }

    public UndefinedSymbol(String message, Throwable cause) {
      super(message, cause);
    }

    public UndefinedSymbol(Throwable cause) {
      super(cause);
    }

    public UndefinedSymbol(
        String message, Throwable cause, boolean enableSuppression, boolean writableStackTrace) {
      super(message, cause, enableSuppression, writableStackTrace);
    }
  }

  @SuppressWarnings("serial")
  public class OutOfBounds extends IndexOutOfBoundsException {}

  @SuppressWarnings("serial")
  public class NoSuchElement extends NoSuchElementException {}

  @SuppressWarnings("serial")
  public class TODO extends RuntimeException {}

  @SuppressWarnings("serial")
  public class Runtime extends RuntimeException {
    public Runtime(String string) {
      super(string);
    }
  }

  @SuppressWarnings("serial")
  public class NULL extends NullPointerException {
    public NULL(String string) {
      super(string);
    }
  }

  @SuppressWarnings({"unchecked"})
  private static <T extends Throwable> void sneakyThrow0(Throwable t) throws T {
    throw (T) t;
  }

  public static RuntimeException Sneaky(Throwable t) {
    if (t == null) throw new NullPointerException();
    sneakyThrow0(t);
    return null;
  }
}
