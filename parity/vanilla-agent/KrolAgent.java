// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A test-only Java agent for the vanilla 1.8.8 server used by the parity suite
// (AGENTS.md section 10.2). It adds one console command, `krolpin <seed>`, which seeds
// every world's random (World.rand) and random block tick generator (World.updateLCG)
// so that behavior driven by them can be compared with krol exactly; krol answers the
// same command in builds with the `parity` feature.
//
// The server jar is obfuscated, so the agent works through reflection on the official
// 1.8.8 names (looked up in the MCP mappings, see reference/tools):
//   net.minecraft.server.MinecraftServer.N()  getServer
//   MinecraftServer.P()                        getCommandManager (an ICommandManager `l`)
//   MinecraftServer.d                          worldServers (WorldServer[])
//   j.a(k)                                     CommandHandler.registerCommand(ICommand)
//   k                                          ICommand
//   adm.s / adm.m                              World.rand / World.updateLCG
//
// Built by `cargo xtask parity` (javac --release 8) and loaded with -javaagent only for
// scenarios that set `pin_random`.

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.Collections;
import java.util.Random;

public final class KrolAgent {
    private static final String NAME = "krolpin";

    private KrolAgent() {}

    public static void premain(String args, Instrumentation inst) {
        Thread t = new Thread(KrolAgent::registerWhenReady, "krol-parity-agent");
        t.setDaemon(true);
        t.start();
    }

    /** Waits for the server object, then adds the command to its command manager. */
    private static void registerWhenReady() {
        try {
            Class<?> serverClass = Class.forName("net.minecraft.server.MinecraftServer");
            Method getServer = serverClass.getMethod("N");
            Method getCommands = serverClass.getMethod("P");
            Object server = null;
            Object commands = null;
            while (commands == null) {
                server = getServer.invoke(null);
                if (server != null) {
                    commands = getCommands.invoke(server);
                }
                if (commands == null) {
                    Thread.sleep(5);
                }
            }
            Class<?> command = Class.forName("k");
            Class<?> handler = Class.forName("j");
            final Object theServer = server;
            InvocationHandler behavior = (proxy, method, a) -> answer(theServer, proxy, method, a);
            Object pin = Proxy.newProxyInstance(command.getClassLoader(), new Class<?>[] {command}, behavior);
            handler.getMethod("a", command).invoke(commands, pin);
        } catch (Exception e) {
            System.err.println("krol parity agent failed: " + e);
        }
    }

    /** The ICommand methods of the pin command, told apart by name and parameters. */
    private static Object answer(Object server, Object self, Method m, Object[] a) throws Exception {
        String n = m.getName();
        int params = m.getParameterCount();
        Class<?> ret = m.getReturnType();
        if (n.equals("c") && params == 0) {
            return NAME; // getCommandName
        }
        if (n.equals("c") && params == 1) {
            return "/" + NAME + " <seed>"; // getCommandUsage
        }
        if (n.equals("b") && params == 0) {
            return Collections.emptyList(); // getCommandAliases
        }
        if (n.equals("a") && params == 1 && ret == boolean.class) {
            return true; // canCommandSenderUseCommand
        }
        if (n.equals("a") && params == 2) {
            pin(server, Long.parseLong(((String[]) a[1])[0])); // processCommand
            return null;
        }
        if (n.equals("a") && params == 3) {
            return null; // addTabCompletionOptions
        }
        if (n.equals("b") && params == 2) {
            return false; // isUsernameIndex
        }
        if (n.equals("compareTo")) {
            Object name = a[0].getClass().getMethod("c").invoke(a[0]);
            return NAME.compareTo((String) name);
        }
        if (n.equals("hashCode")) {
            return System.identityHashCode(self);
        }
        if (n.equals("equals")) {
            return self == a[0];
        }
        if (n.equals("toString")) {
            return NAME;
        }
        throw new UnsupportedOperationException(n);
    }

    /** Seeds World.rand and World.updateLCG of every loaded world. */
    private static void pin(Object server, long seed) throws Exception {
        Field worldsField = server.getClass().getSuperclass().getDeclaredField("d");
        worldsField.setAccessible(true);
        Object[] worlds = (Object[]) worldsField.get(server);
        Class<?> world = Class.forName("adm");
        Field rand = world.getDeclaredField("s");
        rand.setAccessible(true);
        Field lcg = world.getDeclaredField("m");
        lcg.setAccessible(true);
        for (Object w : worlds) {
            if (w == null) {
                continue;
            }
            ((Random) rand.get(w)).setSeed(seed);
            lcg.setInt(w, (int) seed);
        }
    }
}
