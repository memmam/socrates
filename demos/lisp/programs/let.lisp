; let binds locals in parallel: every expr is evaluated in the
; enclosing scope, then the body sees all the names at once.
(define (dist2 x1 y1 x2 y2)
  (let ((dx (- x2 x1))
        (dy (- y2 y1)))
    (+ (* dx dx) (* dy dy))))

(dist2 0 0 3 4)

; parallel, not sequential: this (+ x 1) sees the OUTER x (10),
; not the freshly bound 100.
(define x 10)
(let ((x (* x x))
      (y (+ x 1)))
  (list x y))
