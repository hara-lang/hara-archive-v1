package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icomponent", name = "IComponent")
public interface IComponent {

  @HaraMethod(value = "props", arity = 1)
  IMetadata getProps();

  @HaraMethod(value = "status", arity = 1)
  IMetadata getStatus();

  @HaraMethod(value = "started?", arity = 1)
  boolean isStarted();

  @HaraMethod(value = "stopped?", arity = 1)
  boolean isStopped();

  @HaraMethod(value = "start", arity = 1)
  IComponent start();

  @HaraMethod(value = "stop", arity = 1)
  IComponent stop();

  @HaraMethod(value = "kill", arity = 1)
  default IComponent kill() {
    return this.stop();
  }

  @HaraMethod(value = "remote?", arity = 1)
  default boolean isRemote() {
    return false;
  }
}
