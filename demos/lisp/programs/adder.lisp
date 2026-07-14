; lexical closures: each make-adder call captures its own n
(define (make-adder n)
  (lambda (x) (+ x n)))

(define add5 (make-adder 5))
(define add100 (make-adder 100))

add5
(add5 37)
(add100 (add5 1))
(list (add5 0) (add100 0))
