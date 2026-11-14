// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Prints test vectors for krol's port of vanilla 1.8.8's 2D simplex noise: the biome
// temperature noise (NoiseGeneratorPerlin(new Random(1234), 1)) at a few points, read
// from the SRG-remapped server jar through reflection.
//
// Run: java -cp reference/tools/server-srg.jar tools/datagen/NoiseVectors.java

import java.util.Random;

public class NoiseVectors {
    public static void main(String[] args) throws Exception {
        Class<?> perlin = Class.forName("net.minecraft.world.gen.NoiseGeneratorPerlin");
        Object noise = perlin.getConstructor(Random.class, int.class).newInstance(new Random(1234L), 1);
        Object octaves = perlin.getConstructor(Random.class, int.class).newInstance(new Random(42L), 4);
        double[][] points = {{0, 0}, {1.5, -2.25}, {-58.875, 54.125}, {123.456, 789.012}, {-1e4, 3.3}};
        for (double[] p : points) {
            double a = (Double) perlin.getMethod("func_151601_a", double.class, double.class).invoke(noise, p[0], p[1]);
            double b = (Double) perlin.getMethod("func_151601_a", double.class, double.class).invoke(octaves, p[0], p[1]);
            System.out.println("(" + p[0] + ", " + p[1] + ", " + Long.toHexString(Double.doubleToLongBits(a))
                + ", " + Long.toHexString(Double.doubleToLongBits(b)) + ")");
        }
    }
}
