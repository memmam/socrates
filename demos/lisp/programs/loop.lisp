; a 100000-iteration loop, written as tail recursion. Socrates optimizes
; tail calls, and eval calls itself in tail position, so the optimization
; reaches *through* the interpreter: this runs in constant stack space.
(define (sum-to n acc)
  (if (= n 0)
      acc
      (sum-to (- n 1) (+ acc n))))

(sum-to 100000 0)
