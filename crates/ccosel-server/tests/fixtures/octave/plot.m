% fixture: plot
t = linspace(0, 2*pi, 50);
plot(t, sin(t), t, cos(t));
title("waves");
legend("sin", "cos");
