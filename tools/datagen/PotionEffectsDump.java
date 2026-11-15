// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Dumps the status effects a potion item gives for each damage value
// (PotionHelper.getPotionEffects(damage, false)) from the SRG-remapped server jar (see
// AGENTS.md: reference/tools/server-srg.jar), read through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/PotionEffectsDump.java > potion_effects.txt
//
// Output, one line per damage value with effects, in damage order; consecutive damage
// values with the same effects share a line:
//   <first damage>-<last damage> <effect id>:<duration>:<amplifier> ...

import java.lang.reflect.Method;
import java.util.List;

public class PotionEffectsDump {
    public static void main(String[] args) throws Exception {
        Class.forName("net.minecraft.init.Bootstrap").getMethod("func_151354_b").invoke(null);
        Class<?> helper = Class.forName("net.minecraft.potion.PotionHelper");
        Method effects = helper.getMethod("func_77917_b", int.class, boolean.class);
        Class<?> effect = Class.forName("net.minecraft.potion.PotionEffect");
        Method id = effect.getMethod("func_76456_a");
        Method duration = effect.getMethod("func_76459_b");
        Method amplifier = effect.getMethod("func_76458_c");
        StringBuilder out = new StringBuilder();
        String last = null;
        int first = 0;
        for (int damage = 0; damage <= 32768; damage++) {
            String line = "";
            if (damage <= 32767) {
                List<?> list = (List<?>) effects.invoke(null, damage, false);
                if (list != null) {
                    StringBuilder b = new StringBuilder();
                    for (Object e : list) {
                        b.append(' ').append(id.invoke(e)).append(':').append(duration.invoke(e))
                            .append(':').append(amplifier.invoke(e));
                    }
                    line = b.toString();
                }
            }
            if (!line.equals(last)) {
                if (last != null && !last.isEmpty()) {
                    out.append(first).append('-').append(damage - 1).append(last).append('\n');
                }
                last = line;
                first = damage;
            }
        }
        System.out.print(out);
    }
}
